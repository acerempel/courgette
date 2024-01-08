use std::hash::{Hasher, Hash};
use std::collections::hash_map::DefaultHasher;
use std::fs;
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
fn file_status_changes() {

}

mod parse_makefile {
    use nom::{IResult, multi::separated_list0, bytes::complete::tag};

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
        let rule = Rule {targets, prequisites: todo!(), commands: todo!()};
        Ok((input, rule))
    }
}

mod state {
    use dashmap::DashMap;
    use tokio::sync::Notify;

    pub struct State {
        things: DashMap<Key, Value>,
    }

    enum Value {
        Running(Notify),
        Finished(Result<Rev, eyre::Report>),
    }

    pub(crate) struct Rev(i64);

    pub(crate) struct Key(i64);

    async fn rebuild(state: &State, key: Key) {
        todo!()
    }
}

mod connection_pool {
    use std::ops::{DerefMut, Deref};
    use std::sync::{Arc, Mutex};

    use rusqlite::Connection;
    use tokio::task::spawn_blocking;
    use tokio::sync::{Semaphore, SemaphorePermit};
    use eyre::Report;

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
                })
            }
        }

        async fn new_connection() -> Result<Connection, Report> {
            Ok(spawn_blocking(|| {Connection::open("luna.db") }).await??)
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
