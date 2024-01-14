use std::ops::{Deref, DerefMut};
use std::sync::Mutex;

use eyre::Report;
use rusqlite::Connection;
use tokio::sync::{Semaphore, SemaphorePermit};
use tokio::task::spawn_blocking;

impl Pool {
    fn new(max_conns: usize) -> Self {
        Self {
            semaphore: Semaphore::new(max_conns),
            connections: Mutex::new(Vec::with_capacity(max_conns)),
        }
    }

    async fn new_connection() -> Result<Connection, Report> {
        Ok(spawn_blocking(|| Connection::open("luna.db")).await??)
    }

    fn put_back(&self, conn: Connection) {
        let mut conns = self.connections.lock().unwrap();
        conns.push(conn);
    }

    pub async fn acquire(&self) -> Result<PooledConnection<'_>, Report> {
        let permit = self.semaphore.acquire().await?;
        let conn = {
            let mut conns = self.connections.lock().unwrap();
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

pub struct Pool {
    semaphore: Semaphore,
    connections: Mutex<Vec<Connection>>,
}

pub struct PooledConnection<'a> {
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
