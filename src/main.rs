#![allow(dead_code, unused_variables)]
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;

use eyre::Result;

fn main() -> Result<()> {
    color_eyre::install()?;
    println!("Hello, world!");
    Ok(())
}

struct FileStatus(u64);

impl FileStatus {
    fn get(path: &Path) -> Result<Self> {
        let md = fs::metadata(path)?;
        let size = md.len();
        let mtime = md.modified()?;
        let mut hasher = DefaultHasher::new();
        mtime.hash(&mut hasher);
        size.hash(&mut hasher);
        Ok(Self(hasher.finish()))
    }
}

#[test]
fn file_status_changes() {}

mod parse_makefile {
    use nom::{bytes::complete::tag, multi::separated_list0, IResult};

    type Thing = Vec<u8>;

    pub struct Rule {
        targets: Vec<Thing>,
        prequisites: Vec<Thing>,
        commands: Vec<Thing>,
    }

    fn path(input: &str) -> IResult<&str, Thing> {
        todo!()
    }

    fn rule(input: &str) -> IResult<&str, Rule> {
        let (input, targets) = separated_list0(tag(" "), path)(input)?;
        let rule = Rule {
            targets,
            prequisites: todo!(),
            commands: todo!(),
        };
        Ok((input, rule))
    }
}

mod state {
    use std::any::Any;
    use std::sync::Arc;

    use dashmap::mapref::entry::Entry;
    use dashmap::DashMap;
    use eyre::Report;
    use tokio::sync::Notify;

    pub struct State {
        things: DashMap<Key, Status>,
    }

    impl State {
        pub(crate) async fn wait_for_key(&self, key: Key) -> Result<Rev, Arc<Report>> {
            fn get_result_rev_changed<E: Clone>(res: &Result<Value, E>) -> Result<Rev, E> {
                match res {
                    Ok(status) => Ok(status.changed),
                    Err(err) => Err(err.clone()),
                }
            }
            // Use entry API so that, if the key is missing, we can insert it while retaining a write lock,
            // so no one else can insert it between when we check for it and when we insert it
            let entry = self.things.entry(key);
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
                            let val = self.things.get(&key);
                            match val.as_deref() {
                                Some(Status::Finished(result)) => get_result_rev_changed(result),
                                oh_no => panic!("Oh no! {:?}", oh_no),
                            }
                        }
                        Status::Finished(result) => get_result_rev_changed(result),
                    }
                }
                Entry::Vacant(vac) => {
                    let notify = Arc::new(Notify::new());
                    {
                        // Drop the reference into the map (`VacantEntry::insert` consumes `self`), releasing the lock,
                        // before awaiting
                        vac.insert(Status::Running(notify.clone()));
                    }
                    let result = self.build_key(key).await.map_err(Arc::new);
                    if let Err(ref e) = result {
                        eprintln!("{}", e)
                    }
                    let res = get_result_rev_changed(&result);
                    {
                        // Release the reference into the map (write lock) before waking up dependents
                        *self.things.get_mut(&key).unwrap() = Status::Finished(result);
                    }
                    notify.notify_waiters();
                    res
                }
            }
        }

        async fn build_key(&self, key: Key) -> Result<Value, Report> {
            todo!()
        }

        async fn check_depends(key: Key, last_built: Rev) -> DependencyStatus {
            todo!()
        }
    }

    pub enum DependencyStatus {
        Changed,
        Same,
    }

    #[derive(Debug)]
    enum Status {
        Running(Arc<Notify>),
        Finished(Result<Value, Arc<Report>>),
    }

    #[derive(Debug)]
    struct Value {
        built: Rev,
        changed: Rev,
        value: Box<dyn Any + 'static>,
    }

    #[derive(Debug, Eq, PartialEq, Clone, Copy)]
    pub(crate) struct Rev(i64);

    #[derive(Hash, Eq, PartialEq, Clone, Debug, Copy)]
    pub(crate) struct Key(i64);
}

mod connection_pool {
    use std::ops::{Deref, DerefMut};
    use std::sync::{Arc, Mutex};

    use eyre::Report;
    use rusqlite::Connection;
    use tokio::sync::{Semaphore, SemaphorePermit};
    use tokio::task::spawn_blocking;

    #[derive(Clone)]
    pub(crate) struct Pool {
        inner: Arc<Inner>,
    }

    impl Pool {
        fn new(max_conns: usize) -> Self {
            Self {
                inner: Arc::new(Inner {
                    semaphore: Semaphore::new(max_conns),
                    connections: Mutex::new(Vec::with_capacity(max_conns)),
                }),
            }
        }

        async fn new_connection() -> Result<Connection, Report> {
            Ok(spawn_blocking(|| Connection::open("luna.db")).await??)
        }

        fn put_back(&self, conn: Connection) {
            let mut conns = self.inner.connections.lock().unwrap();
            conns.push(conn);
        }

        pub(crate) async fn acquire(&self) -> Result<PooledConnection<'_>, Report> {
            let permit = self.inner.semaphore.acquire().await?;
            let conn = {
                let mut conns = self.inner.connections.lock().unwrap();
                if let Some(conn) = conns.pop() {
                    conn
                } else {
                    drop(conns);
                    Self::new_connection().await?
                }
            };
            let pconn = PooledConnection {
                connection: Some(conn),
                permit,
                pool: self,
            };
            Ok(pconn)
        }
    }

    struct Inner {
        semaphore: Semaphore,
        connections: Mutex<Vec<Connection>>,
    }

    pub(crate) struct PooledConnection<'a> {
        connection: Option<Connection>,
        permit: SemaphorePermit<'a>,
        pool: &'a Pool,
    }

    impl<'a> Drop for PooledConnection<'a> {
        fn drop(&mut self) {
            self.pool.put_back(self.connection.take().unwrap());
        }
    }

    impl<'a> Deref for PooledConnection<'a> {
        type Target = Connection;

        fn deref(&self) -> &Self::Target {
            self.connection.as_ref().unwrap()
        }
    }

    impl<'a> DerefMut for PooledConnection<'a> {
        fn deref_mut(&mut self) -> &mut Self::Target {
            self.connection.as_mut().unwrap()
        }
    }
}
