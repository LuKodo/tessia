//! Throwaway Postgres databases for tests.
//!
//! [`TestDb::new`] creates a database with a random name on the server named
//! by `TESSIA_TEST_PG_ADMIN_URL` (default
//! `postgres://postgres:postgres@127.0.0.1:55432/postgres`), applies all
//! migrations and drops the database when the value is dropped. Each test
//! gets its own database, so tests run in parallel without sharing state.
//!
//! Available to unit tests, and to `tests/*.rs` through the `test-support`
//! feature (enabled automatically by the self dev-dependency).
//!
//! Keep pools small in tests: the test server allows ~100 connections and
//! several test binaries (and other writers) share it.

use diesel::pg::PgConnection;
use diesel::{Connection, RunQueryDsl, sql_query};

use super::migrate;
use super::pool::{Conn, DbPool};

/// Default admin URL of the TEST Postgres container.
pub const DEFAULT_ADMIN_URL: &str = "postgres://postgres:postgres@127.0.0.1:55432/postgres";

/// Pool size of a test database.
const POOL_SIZE: u32 = 6;

/// A migrated, randomly named database that is dropped on `Drop`.
pub struct TestDb {
    admin_url: String,
    name: String,
    url: String,
    pool: Option<DbPool>,
}

fn admin_url() -> String {
    std::env::var("TESSIA_TEST_PG_ADMIN_URL").unwrap_or_else(|_| DEFAULT_ADMIN_URL.to_owned())
}

/// Replaces the database name (last path segment) of a Postgres URL.
fn with_database(url: &str, database: &str) -> anyhow::Result<String> {
    let mut parsed = url::Url::parse(url)?;
    parsed.set_path(&format!("/{database}"));
    Ok(parsed.to_string())
}

impl TestDb {
    /// Creates and migrates a throwaway database. Panics with a clear message
    /// when the test server is unreachable.
    pub fn new() -> TestDb {
        match Self::try_new() {
            Ok(db) => db,
            Err(err) => panic!(
                "no se pudo crear la base de datos de prueba (¿está arriba Postgres de prueba? TESSIA_TEST_PG_ADMIN_URL): {err:#}"
            ),
        }
    }

    /// Fallible [`TestDb::new`].
    pub fn try_new() -> anyhow::Result<TestDb> {
        let admin_url = admin_url();
        let name = format!("tessia_test_{}", uuid::Uuid::new_v4().simple());
        let mut admin = PgConnection::establish(&admin_url)?;
        sql_query(format!("CREATE DATABASE \"{name}\"")).execute(&mut admin)?;
        let url = with_database(&admin_url, &name)?;
        let mut db = TestDb {
            admin_url,
            name,
            url,
            pool: None,
        };
        migrate::migrate_url(&db.url)?;
        db.pool = Some(DbPool::connect(&db.url, POOL_SIZE)?);
        Ok(db)
    }

    /// Connection URL of this database.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Name of this database.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The pool (cloneable; share it with the code under test).
    pub fn pool(&self) -> DbPool {
        match &self.pool {
            Some(pool) => pool.clone(),
            None => unreachable!("the pool is set by try_new"),
        }
    }

    /// A pooled connection for direct SQL in tests.
    pub fn conn(&self) -> Conn {
        match self.pool().get() {
            Ok(conn) => conn,
            Err(err) => panic!("no hay conexión de prueba: {err}"),
        }
    }
}

impl Default for TestDb {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        // Close our connections first or DROP DATABASE would have to evict them.
        self.pool = None;
        if let Ok(mut admin) = PgConnection::establish(&self.admin_url) {
            let _ = sql_query(format!(
                "DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)",
                self.name
            ))
            .execute(&mut admin);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_name_is_replaced_in_the_url() {
        let url = with_database("postgres://u:p@h:1/postgres?sslmode=disable", "x").expect("url");
        assert_eq!(url, "postgres://u:p@h:1/x?sslmode=disable");
    }

    #[test]
    fn creates_a_migrated_database_and_drops_it() {
        let name;
        {
            let db = TestDb::new();
            name = db.name().to_owned();
            let mut conn = db.conn();
            let tables: Vec<Table> = sql_query(
                "SELECT tablename::text AS name FROM pg_tables WHERE schemaname = 'public'",
            )
            .load(&mut conn)
            .expect("list tables");
            assert!(
                tables.iter().any(|t| t.name == "proceso_estado"),
                "{:?}",
                tables.len()
            );
        }
        let mut admin = PgConnection::establish(&admin_url()).expect("admin");
        let left: Vec<Table> =
            sql_query("SELECT datname::text AS name FROM pg_database WHERE datname = $1")
                .bind::<diesel::sql_types::Text, _>(&name)
                .load(&mut admin)
                .expect("list databases");
        assert!(left.is_empty());
    }

    #[derive(diesel::QueryableByName)]
    struct Table {
        #[diesel(sql_type = diesel::sql_types::Text)]
        name: String,
    }
}
