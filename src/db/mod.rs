//! Postgres access: pool, retry, embedded migrations and test support.
//!
//! ```no_run
//! use tessia::db::{DbPool, DbResult};
//! use diesel::prelude::*;
//!
//! fn demo(pool: &DbPool) -> DbResult<()> {
//!     // One idempotent statement, retried on transient errors.
//!     pool.run("ping", |conn| {
//!         diesel::sql_query("SELECT 1").execute(conn)?;
//!         Ok(())
//!     })?;
//!     // A unit of work in one transaction (re-run as a whole on deadlock).
//!     pool.transaction("demo", |conn| {
//!         diesel::sql_query("SELECT 1").execute(conn)?;
//!         Ok(())
//!     })
//! }
//! ```

pub mod cli;
mod error;
pub mod migrate;
mod pool;
mod retry;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use error::{DbError, DbResult};
pub use pool::{Conn, DbPool};
pub use retry::{MAX_ATTEMPTS, backoff, with_retry};
