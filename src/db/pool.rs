//! Connection pool.

use std::time::Duration;

use diesel::Connection;
use diesel::pg::PgConnection;
use diesel::r2d2::{ConnectionManager, Pool, PooledConnection};

use super::error::{DbError, DbResult};
use super::retry::with_retry;
use crate::config::Settings;

/// A connection checked out of the pool; derefs to [`PgConnection`].
pub type Conn = PooledConnection<ConnectionManager<PgConnection>>;

/// Cloneable handle to the Postgres pool (an `Arc` inside).
#[derive(Clone)]
pub struct DbPool {
    inner: Pool<ConnectionManager<PgConnection>>,
}

impl std::fmt::Debug for DbPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.inner.state();
        f.debug_struct("DbPool")
            .field("connections", &state.connections)
            .field("idle", &state.idle_connections)
            .finish()
    }
}

impl DbPool {
    /// Opens a pool of at most `max_size` connections to `url` and verifies
    /// that one connection can be made. The URL (it has a password) is never
    /// echoed in errors.
    pub fn connect(url: &str, max_size: u32) -> DbResult<DbPool> {
        let manager = ConnectionManager::<PgConnection>::new(url);
        let inner = Pool::builder()
            .max_size(max_size.max(1))
            .min_idle(Some(0))
            .connection_timeout(Duration::from_secs(30))
            .test_on_check_out(true)
            .build(manager)?;
        let pool = DbPool { inner };
        drop(pool.get()?);
        Ok(pool)
    }

    /// Builds the pool from `database.url` and `database.pool_size`.
    pub fn from_settings(settings: &Settings) -> anyhow::Result<DbPool> {
        let url = settings.require_one("database.url")?;
        let size = settings
            .get("database.pool_size")
            .and_then(|v| v.parse().ok())
            .unwrap_or(16);
        Ok(DbPool::connect(url, size)?)
    }

    /// Checks a connection out of the pool.
    pub fn get(&self) -> DbResult<Conn> {
        Ok(self.inner.get()?)
    }

    /// Runs `unit` on a pooled connection, running it again on a fresh checkout
    /// after a transient failure (see [`with_retry`]). Use for single
    /// idempotent statements (upserts, reads, compare-and-set updates).
    pub fn run<T>(
        &self,
        op: &str,
        mut unit: impl FnMut(&mut PgConnection) -> DbResult<T>,
    ) -> DbResult<T> {
        with_retry(op, || {
            let mut conn = self.get()?;
            unit(&mut conn)
        })
    }

    /// Runs `unit` in one transaction on a pooled connection, committing when
    /// it returns `Ok` and rolling back on `Err`. After a transient failure
    /// (deadlock, serialization failure, lost connection) the whole unit runs
    /// again in a new transaction, so it must only touch the database through
    /// its argument and must not have other side effects.
    pub fn transaction<T>(
        &self,
        op: &str,
        mut unit: impl FnMut(&mut PgConnection) -> DbResult<T>,
    ) -> DbResult<T> {
        with_retry(op, || {
            let mut conn = self.get()?;
            conn.transaction::<T, DbError, _>(|conn| unit(conn))
        })
    }
}
