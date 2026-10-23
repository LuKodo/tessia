//! `leases`: named, expiring, cross-host mutual exclusion (replaces the Go
//! `.tessia-*.lock` files).
//!
//! A lease is held by `owner` until `expires_at` (database clock). Another
//! owner can acquire it only after it expires or was released. Holders that
//! work longer than the TTL call [`LeaseStore::renew`] periodically; a holder
//! that crashes simply stops renewing and the lease frees itself.
//!
//! ```no_run
//! # use tessia::{db::DbPool, stores::leases::LeaseStore};
//! # use std::time::Duration;
//! # fn demo(pool: DbPool) -> anyhow::Result<()> {
//! let leases = LeaseStore::new(pool);
//! if let Some(guard) = leases.guard("diario:2026-10-21", "host:1:uuid", Duration::from_secs(600))? {
//!     // exclusive section; guard.renew()? for long work
//!     drop(guard); // released
//! }
//! # Ok(()) }
//! ```

mod schema;

use std::time::Duration;

use diesel::dsl::sql;
use diesel::prelude::*;
use diesel::sql_types::{Bool, Double, Text};

use crate::db::{DbPool, DbResult};
use schema::leases as t;

/// Predicate: the lease is no longer held.
fn expired() -> diesel::expression::SqlLiteral<Bool> {
    sql::<Bool>("leases.expires_at <= now()")
}

fn expiry(ttl: Duration) -> diesel::expression::SqlLiteral<diesel::sql_types::Timestamptz> {
    // `secs` comes from a Duration (a plain number), never from user text.
    sql::<diesel::sql_types::Timestamptz>(&format!(
        "now() + make_interval(secs => {:.3})",
        ttl.as_secs_f64()
    ))
}

#[derive(Clone)]
pub struct LeaseStore {
    pool: DbPool,
}

impl LeaseStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Takes the lease `name` for `ttl` when it is free, expired, or already
    /// ours (which extends it). False when another owner holds it.
    pub fn acquire(&self, name: &str, owner: &str, ttl: Duration) -> DbResult<bool> {
        self.pool.run("acquire lease", |conn| {
            let rows = diesel::sql_query(
                "INSERT INTO leases (name, owner, expires_at) \
                 VALUES ($1, $2, now() + make_interval(secs => $3)) \
                 ON CONFLICT (name) DO UPDATE \
                 SET owner = EXCLUDED.owner, expires_at = EXCLUDED.expires_at, updated_at = now() \
                 WHERE leases.expires_at <= now() OR leases.owner = EXCLUDED.owner",
            )
            .bind::<Text, _>(name)
            .bind::<Text, _>(owner)
            .bind::<Double, _>(ttl.as_secs_f64())
            .execute(conn)?;
            Ok(rows > 0)
        })
    }

    /// Extends our lease by `ttl` from now. False when we no longer own it
    /// (it expired and another owner took it, or it was released).
    pub fn renew(&self, name: &str, owner: &str, ttl: Duration) -> DbResult<bool> {
        self.pool.run("renew lease", |conn| {
            let rows = diesel::update(t::table.filter(t::name.eq(name).and(t::owner.eq(owner))))
                .set((
                    t::expires_at.eq(expiry(ttl)),
                    t::updated_at.eq(diesel::dsl::now),
                ))
                .execute(conn)?;
            Ok(rows > 0)
        })
    }

    /// Releases our lease; false when we did not hold it.
    pub fn release(&self, name: &str, owner: &str) -> DbResult<bool> {
        self.pool.run("release lease", |conn| {
            Ok(
                diesel::delete(t::table.filter(t::name.eq(name).and(t::owner.eq(owner))))
                    .execute(conn)?
                    > 0,
            )
        })
    }

    /// The current holder of `name`, if the lease has not expired.
    pub fn holder(&self, name: &str) -> DbResult<Option<String>> {
        self.pool.run("lease holder", |conn| {
            Ok(t::table
                .filter(t::name.eq(name).and(diesel::dsl::not(expired())))
                .select(t::owner)
                .first(conn)
                .optional()?)
        })
    }

    /// Acquires the lease and returns a guard that releases it on drop, or
    /// `None` when another owner holds it.
    pub fn guard(&self, name: &str, owner: &str, ttl: Duration) -> DbResult<Option<LeaseGuard>> {
        if !self.acquire(name, owner, ttl)? {
            return Ok(None);
        }
        Ok(Some(LeaseGuard {
            store: self.clone(),
            name: name.to_owned(),
            owner: owner.to_owned(),
            ttl,
        }))
    }
}

/// A held lease; released when dropped (best effort: if the release fails the
/// lease simply expires).
pub struct LeaseGuard {
    store: LeaseStore,
    name: String,
    owner: String,
    ttl: Duration,
}

impl LeaseGuard {
    /// Extends the lease by its original TTL; false when it was lost.
    pub fn renew(&self) -> DbResult<bool> {
        self.store.renew(&self.name, &self.owner, self.ttl)
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        if let Err(err) = self.store.release(&self.name, &self.owner) {
            tracing::warn!(lease = %self.name, error = %err, "could not release lease; it will expire");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};
    use std::thread;

    use super::*;
    use crate::db::test_support::TestDb;

    const LONG: Duration = Duration::from_secs(60);

    #[test]
    fn only_one_owner_holds_a_lease_and_it_can_be_released() {
        let db = TestDb::new();
        let leases = LeaseStore::new(db.pool());
        assert!(leases.acquire("x", "a", LONG).expect("a"));
        assert!(!leases.acquire("x", "b", LONG).expect("b blocked"));
        assert!(leases.acquire("x", "a", LONG).expect("a re-acquires"));
        assert_eq!(leases.holder("x").expect("holder").as_deref(), Some("a"));
        assert!(!leases.release("x", "b").expect("b cannot release"));
        assert!(leases.release("x", "a").expect("a releases"));
        assert!(leases.acquire("x", "b", LONG).expect("b takes it"));
        assert!(
            leases
                .acquire("other", "a", LONG)
                .expect("independent names")
        );
    }

    #[test]
    fn expired_leases_are_taken_over_and_renewal_keeps_them() {
        let db = TestDb::new();
        let leases = LeaseStore::new(db.pool());
        assert!(
            leases
                .acquire("x", "a", Duration::from_millis(50))
                .expect("a")
        );
        thread::sleep(Duration::from_millis(120));
        assert_eq!(leases.holder("x").expect("expired"), None);
        assert!(leases.acquire("x", "b", LONG).expect("takeover"));
        assert!(!leases.renew("x", "a", LONG).expect("a lost it"));
        assert!(leases.renew("x", "b", LONG).expect("b renews"));
    }

    #[test]
    fn guard_releases_on_drop() {
        let db = TestDb::new();
        let leases = LeaseStore::new(db.pool());
        {
            let guard = leases.guard("x", "a", LONG).expect("guard").expect("held");
            assert!(leases.guard("x", "b", LONG).expect("second").is_none());
            assert!(guard.renew().expect("renew"));
        }
        assert!(leases.guard("x", "b", LONG).expect("after drop").is_some());
    }

    #[test]
    fn racing_acquirers_have_a_single_winner() {
        let db = TestDb::new();
        let leases = LeaseStore::new(db.pool());
        let barrier = Arc::new(Barrier::new(6));
        let handles: Vec<_> = (0..6)
            .map(|n| {
                let (leases, barrier) = (leases.clone(), barrier.clone());
                thread::spawn(move || {
                    barrier.wait();
                    leases
                        .acquire("race", &format!("w{n}"), LONG)
                        .expect("acquire")
                })
            })
            .collect();
        let winners = handles
            .into_iter()
            .filter(|_| true)
            .map(|h| h.join().expect("worker"))
            .filter(|won| *won)
            .count();
        assert_eq!(winners, 1);
    }
}
