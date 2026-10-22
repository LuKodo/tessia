//! Embedded migrations, serialized across machines with an advisory lock.

use diesel::pg::PgConnection;
use diesel::sql_query;
use diesel::{Connection, RunQueryDsl};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};

use super::error::{DbError, DbResult};

/// Every migration under `migrations/`, embedded in the binary.
pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

/// Key of the Postgres advisory lock that serializes migrations
/// (the ASCII bytes of "tessia").
const ADVISORY_LOCK_KEY: i64 = 0x7465_7373_6961;

/// Applies the pending migrations on `conn` and returns their versions.
///
/// Several machines may start at the same time: a session-level advisory lock
/// makes the later ones wait, then find nothing pending. Calling it again is a
/// no-op. The lock is released before returning (and by Postgres if the
/// connection dies).
pub fn run_pending(conn: &mut PgConnection) -> DbResult<Vec<String>> {
    sql_query(format!("SELECT pg_advisory_lock({ADVISORY_LOCK_KEY})")).execute(conn)?;
    let applied = conn
        .run_pending_migrations(MIGRATIONS)
        .map(|versions| versions.iter().map(ToString::to_string).collect::<Vec<_>>())
        .map_err(|err| DbError::other(format!("migraciones: {err}")));
    let unlock = sql_query(format!("SELECT pg_advisory_unlock({ADVISORY_LOCK_KEY})")).execute(conn);
    let applied = applied?;
    unlock?;
    Ok(applied)
}

/// Opens one connection to `url` and applies the pending migrations.
pub fn migrate_url(url: &str) -> DbResult<Vec<String>> {
    let mut conn = PgConnection::establish(url)?;
    run_pending(&mut conn)
}

/// Number of migrations embedded in this binary.
pub fn embedded_count() -> usize {
    use diesel::migration::MigrationSource;
    MigrationSource::<diesel::pg::Pg>::migrations(&MIGRATIONS)
        .map(|m| m.len())
        .unwrap_or(0)
}
