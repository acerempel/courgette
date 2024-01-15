use std::any::Any;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use eyre::Report;
use tokio::sync::{oneshot, Notify};
use tokio::task::JoinSet;

use crate::connection_pool::Pool;

#[derive(Clone)]
pub struct Shared {
    inner: Arc<Inner>,
    current_rev: Rev,
}

struct Inner {
    things: DashMap<Key, Status>,
    pool: Pool,
}

#[derive(Debug, Clone, Copy)]
struct Failed;

enum BuildResult {
    DependencyFailed,
    Completed(Result<(Answer, Thing, Vec<Vec<Key>>), Report>),
}

type Thing = Box<dyn Any + Send + Sync + 'static>;

enum Answer {
    DidNotNeedToRecompute,
    RecomputedDifferent(Vec<u8>),
    RecomputedSame,
}

struct Stored {
    built: Rev,
    changed: Rev,
    kty: KeyTypeId,
    failed: bool,
    witness: Vec<u8>,
    depends: Vec<Vec<Key>>,
}

#[derive(Debug, Clone, Copy)]
struct KeyTypeId(i64);

impl Shared {
    fn things(&self) -> &DashMap<Key, Status> {
        &self.inner.things
    }
    pub async fn wait_for_key(&self, key: Key) -> Result<Value, ()> {
        fn get_result_rev_changed<E: Clone>(res: &Result<Value, E>) -> Result<Rev, E> {
            match res {
                Ok(status) => Ok(status.changed),
                Err(err) => Err(err.clone()),
            }
        }
        // Use entry API so that, if the key is missing, we can insert it while retaining a write lock,
        // so no one else can insert it between when we check for it and when we insert it
        let entry = self.things().entry(key);
        match entry {
            Entry::Occupied(occ) => {
                // We don't need a write lock if the key exists already
                let occref = occ.into_ref().downgrade();
                match occref.value() {
                    Status::Running(ref notify) => {
                        let notify = notify.clone();
                        // Drop the reference into the map, releasing the lock, before awaiting
                        drop(occref);
                        notify.notified().await;
                        let val = self.things().get(&key);
                        match val.as_deref() {
                            Some(Status::Finished(result)) => {
                                result.as_ref().cloned().map_err(|_| ())
                            }
                            oh_no => panic!("Oh no! {:?}", oh_no),
                        }
                    }
                    Status::Finished(ref result) => result.as_ref().cloned().map_err(|_| ()),
                }
            }
            Entry::Vacant(vac) => {
                let notify = Arc::new(Notify::new());
                {
                    // Drop the reference into the map (`VacantEntry::insert` consumes `self`), releasing the lock,
                    // before awaiting. -- Is this necessary, though? When exactly does the result of calling this
                    // function get dropped (given that it's not stored in a variable or a temporary or passed by
                    // value into another function?)
                    vac.insert(Status::Running(notify.clone()));
                }
                let result = self.update_key(key).await;
                let res = result.as_ref().cloned().map_err(|_| ());
                {
                    // Release the reference into the map (write lock) before waking up dependents. Again, it's
                    // not clear to me whether introducing a new scope here is actually necessary to do this.
                    *self.things().get_mut(&key).unwrap() = Status::Finished(result);
                }
                notify.notify_waiters();
                res
            }
        }
    }

    async fn update_key(&self, key: Key) -> Result<Value, Report> {
        let old_stored = self
            .get_stored(key)
            .await
            .expect("error fetching key from database");
        let result = self.build_key(key, &old_stored).await;
        match result {
            BuildResult::DependencyFailed => Err(Report::msg("dependency failed")),
            BuildResult::Completed(result) => {
                let (val, new_stored) = match result {
                    Ok((answer, thing, depends)) => match answer {
                        Answer::RecomputedDifferent(witness) => (
                            Ok(Value {
                                changed: self.current_rev,
                                value: thing.into(),
                            }),
                            Stored {
                                witness,
                                changed: self.current_rev,
                                built: self.current_rev,
                                kty: old_stored.kty,
                                failed: false,
                                depends,
                            },
                        ),
                        Answer::RecomputedSame => (
                            Ok(Value {
                                changed: old_stored.changed,
                                value: thing.into(),
                            }),
                            Stored {
                                failed: false,
                                built: self.current_rev,
                                depends,
                                ..old_stored
                            },
                        ),
                        Answer::DidNotNeedToRecompute => (
                            Ok(Value {
                                changed: old_stored.changed,
                                value: thing.into(),
                            }),
                            Stored {
                                failed: false,
                                built: self.current_rev,
                                ..old_stored
                            },
                        ),
                    },
                    Err(err) => {
                        eprintln!("{}", err);
                        (
                            Err(err),
                            Stored {
                                failed: true,
                                ..old_stored
                            },
                        )
                    }
                };
                self.put_stored(new_stored)
                    .await
                    .expect("error writing to database");
                val
            }
        }
    }

    async fn build_key(&self, key: Key, stored: &Stored) -> BuildResult {
        let status = self.check_depends(key, stored).await;
        match status {
            DependencyStatus::Failed => BuildResult::DependencyFailed,
            DependencyStatus::Changed | DependencyStatus::Same => {
                let (sender, recv) = oneshot::channel();
                let mut ctx = Context::new(sender);
                let builder = self.get_builder(stored.kty)(&mut ctx, &stored.witness, status);
                let result = tokio::select! {
                    completed = builder => BuildResult::Completed(completed.map(|(thing, ans)| (thing, ans, ctx.depends))),
                    dep_failed = recv => BuildResult::DependencyFailed,
                };
                result
            }
        }
    }

    /// Returns a boxed future to break the cycle of recursive async functions
    /// (so that it does not give rise to a cycle of recursive types).
    fn check_depends<'a>(
        &self,
        key: Key,
        stored: &'a Stored,
    ) -> Pin<Box<dyn Future<Output = DependencyStatus> + Send + 'a>> {
        let shared = self.clone();
        Box::pin(async move {
            for par_deps in &stored.depends {
                let mut dep_tasks = JoinSet::new();
                for dep in par_deps {
                    let shared = shared.clone();
                    let dep = *dep;
                    dep_tasks.spawn(async move { shared.wait_for_key(dep).await });
                }
                while let Some(result) = dep_tasks.join_next().await {
                    let value = result.expect("could not join dependency task");
                    match value {
                        Ok(value) => {
                            if value.changed > stored.built {
                                dep_tasks.detach_all();
                                return DependencyStatus::Changed;
                            }
                        }
                        Err(_) => {
                            dep_tasks.detach_all();
                            return DependencyStatus::Failed;
                        }
                    }
                }
            }
            DependencyStatus::Same
        })
    }

    async fn get_depends(&self, key: Key) -> Result<Vec<Key>, Report> {
        todo!()
    }

    async fn get_stored(&self, key: Key) -> Result<Stored, Report> {
        todo!()
    }

    async fn put_stored(&self, new_stored: Stored) -> Result<(), Report> {
        todo!()
    }

    fn get_builder(
        &self,
        kty: KeyTypeId,
    ) -> Box<
        dyn FnMut(
            &mut Context,
            &[u8],
            DependencyStatus,
        ) -> Pin<Box<dyn Future<Output = Result<(Answer, Thing), Report>> + Send>>,
    > {
        todo!()
    }
}

pub struct Context {
    sender: oneshot::Sender<()>,
    depends: Vec<Vec<Key>>,
}

impl Context {
    pub fn new(sender: oneshot::Sender<()>) -> Self {
        Self {
            sender,
            depends: Vec::new(),
        }
    }
}

pub enum DependencyStatus {
    Changed,
    Same,
    Failed,
}

#[derive(Debug)]
enum Status {
    Running(Arc<Notify>),
    Finished(Result<Value, Report>),
}

pub struct DependencyFailed;

#[derive(Debug, Clone)]
pub struct Value {
    changed: Rev,
    value: Arc<dyn Any + Send + Sync + 'static>,
}

#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub struct Rev(i64);

impl PartialOrd for Rev {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.0.partial_cmp(&other.0)
    }
}

#[derive(Hash, Eq, PartialEq, Clone, Debug, Copy)]
pub struct Key(i64);
