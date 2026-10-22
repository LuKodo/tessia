//! `cargues`: one row per uploaded convenio CSV.
//!
//! A *load* is identified by its id (the CSV stem, see
//! [`crate::domain::cargue`]). The same load can receive different bytes over
//! time; each distinct `(id, sha256)` is one row, and the *current* upload of a
//! load is the newest one. Re-ingesting bytes that are not the newest fails
//! with [`IngestError::Superseded`] (Go `ErrCargueSuperseded`): a preparation
//! works on the newest upload.
//!
//! ```no_run
//! # use tessia::{db::DbPool, stores::cargues::{CarguesStore, NewCargue}};
//! # fn demo(pool: DbPool) -> anyhow::Result<()> {
//! let store = CarguesStore::new(pool);
//! let info = tessia::domain::cargue::load_context("Convenio_21102026_21102026.csv");
//! let sha = "0".repeat(64);
//! let ingested = store.ingest(&NewCargue::from_load(&info, &sha))?;
//! assert!(ingested.created);
//! # Ok(()) }
//! ```

mod schema;

use chrono::{DateTime, NaiveDate, Utc};
use diesel::prelude::*;

use crate::db::{DbError, DbPool, DbResult};
use crate::domain::cargue::{LoadInfo, LoadOrigin, is_valid_id};
use schema::cargues;

/// A stored upload.
#[derive(Debug, Clone, PartialEq, Eq, Queryable, Selectable)]
#[diesel(table_name = cargues)]
pub struct Cargue {
    /// Upload sequence; higher is newer.
    pub seq: i64,
    pub id: String,
    pub sha256: String,
    pub name: String,
    origin: String,
    pub date_from: Option<NaiveDate>,
    pub date_to: Option<NaiveDate>,
    pub created_at: DateTime<Utc>,
}

impl Cargue {
    pub fn origin(&self) -> LoadOrigin {
        LoadOrigin::parse(&self.origin).unwrap_or(LoadOrigin::Unknown)
    }
}

/// An upload to register.
#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = cargues)]
pub struct NewCargue {
    pub id: String,
    pub sha256: String,
    pub name: String,
    origin: String,
    pub date_from: Option<NaiveDate>,
    pub date_to: Option<NaiveDate>,
}

impl NewCargue {
    /// Builds the row from the identity derived from the file name and the
    /// lowercase-hex SHA-256 of the CSV bytes.
    pub fn from_load(info: &LoadInfo, sha256: &str) -> Self {
        Self {
            id: info.id.clone(),
            sha256: sha256.to_owned(),
            name: info.name.clone(),
            origin: info.origin.as_str().to_owned(),
            date_from: info.from,
            date_to: info.to,
        }
    }

    /// Overrides the provenance.
    #[must_use]
    pub fn with_origin(mut self, origin: LoadOrigin) -> Self {
        self.origin = origin.as_str().to_owned();
        self
    }
}

/// Result of [`CarguesStore::ingest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ingested {
    pub cargue: Cargue,
    /// False when the same `(id, sha256)` was already stored.
    pub created: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error(
        "ya existe un cargue más reciente de «{id}» (#{newest}); este CSV es el #{seq} y los lectores usan el más reciente: prepare el archivo más reciente"
    )]
    Superseded { id: String, seq: i64, newest: i64 },
    #[error("cargue inválido: {0}")]
    Invalid(String),
    #[error(transparent)]
    Db(#[from] DbError),
}

pub(crate) fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[derive(Clone)]
pub struct CarguesStore {
    pool: DbPool,
}

impl CarguesStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Registers an upload. The same `(id, sha256)` returns the stored row with
    /// `created = false` (also when two machines race). New bytes under a known
    /// id are a new, newer row. Fails with [`IngestError::Superseded`] when the
    /// `(id, sha256)` already exists but a newer upload of the load exists.
    pub fn ingest(&self, new: &NewCargue) -> Result<Ingested, IngestError> {
        if !is_valid_id(&new.id) {
            return Err(IngestError::Invalid(format!(
                "id de cargue no válido: {:?}",
                new.id
            )));
        }
        if !is_sha256_hex(&new.sha256) {
            return Err(IngestError::Invalid(
                "el SHA-256 debe ser hexadecimal en minúsculas de 64 caracteres".into(),
            ));
        }
        let (cargue, created, newest) = self.pool.run("ingest cargue", |conn| {
            let inserted = diesel::insert_into(cargues::table)
                .values(new)
                .on_conflict((cargues::id, cargues::sha256))
                .do_nothing()
                .returning(Cargue::as_returning())
                .get_result(conn)
                .optional()?;
            let (cargue, created) = match inserted {
                Some(row) => (row, true),
                None => (
                    cargues::table
                        .filter(cargues::id.eq(&new.id).and(cargues::sha256.eq(&new.sha256)))
                        .select(Cargue::as_select())
                        .first(conn)?,
                    false,
                ),
            };
            let newest = newest_seq(conn, &new.id)?.unwrap_or(cargue.seq);
            Ok((cargue, created, newest))
        })?;
        if cargue.seq != newest {
            return Err(IngestError::Superseded {
                id: cargue.id,
                seq: cargue.seq,
                newest,
            });
        }
        Ok(Ingested { cargue, created })
    }

    /// The newest upload of the load `id`.
    pub fn newest(&self, id: &str) -> DbResult<Option<Cargue>> {
        self.pool.run("newest cargue", |conn| {
            Ok(cargues::table
                .filter(cargues::id.eq(id))
                .order(cargues::seq.desc())
                .select(Cargue::as_select())
                .first(conn)
                .optional()?)
        })
    }

    /// The upload of load `id` with exactly these bytes.
    pub fn get(&self, id: &str, sha256: &str) -> DbResult<Option<Cargue>> {
        self.pool.run("get cargue", |conn| {
            Ok(cargues::table
                .filter(cargues::id.eq(id).and(cargues::sha256.eq(sha256)))
                .select(Cargue::as_select())
                .first(conn)
                .optional()?)
        })
    }

    /// The newest upload of every load created at or after `since`, newest
    /// first (at most `limit`).
    pub fn recent(&self, since: DateTime<Utc>, limit: i64) -> DbResult<Vec<Cargue>> {
        self.pool.run("recent cargues", |conn| {
            Ok(cargues::table
                .filter(cargues::created_at.ge(since))
                .filter(diesel::dsl::sql::<diesel::sql_types::Bool>(
                    "seq = (SELECT MAX(c.seq) FROM cargues c WHERE c.id = cargues.id)",
                ))
                .order(cargues::seq.desc())
                .limit(limit)
                .select(Cargue::as_select())
                .load(conn)?)
        })
    }
}

fn newest_seq(conn: &mut PgConnection, id: &str) -> DbResult<Option<i64>> {
    use diesel::dsl::max;
    Ok(cargues::table
        .filter(cargues::id.eq(id))
        .select(max(cargues::seq))
        .first(conn)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::TestDb;
    use crate::domain::cargue::load_context;

    fn sha(c: char) -> String {
        c.to_string().repeat(64)
    }

    fn row(name: &str, hash: char) -> NewCargue {
        NewCargue::from_load(&load_context(name), &sha(hash))
    }

    #[test]
    fn same_bytes_return_the_stored_row() {
        let db = TestDb::new();
        let store = CarguesStore::new(db.pool());
        let first = store
            .ingest(&row("Convenio_21102026_21102026.csv", 'a'))
            .expect("first");
        let again = store
            .ingest(&row("Convenio_21102026_21102026.csv", 'a'))
            .expect("again");
        assert!(first.created && !again.created);
        assert_eq!(first.cargue, again.cargue);
        assert_eq!(
            first.cargue.date_from.map(|d| d.to_string()).as_deref(),
            Some("2026-10-21")
        );
        assert_eq!(first.cargue.origin(), LoadOrigin::Unknown);
    }

    #[test]
    fn newer_bytes_supersede_and_older_bytes_are_refused() {
        let db = TestDb::new();
        let store = CarguesStore::new(db.pool());
        let old = store.ingest(&row("Manual_1_abc.csv", 'a')).expect("old");
        let new = store.ingest(&row("Manual_1_abc.csv", 'b')).expect("new");
        assert!(new.cargue.seq > old.cargue.seq);
        match store.ingest(&row("Manual_1_abc.csv", 'a')) {
            Err(IngestError::Superseded { seq, newest, .. }) => {
                assert_eq!((seq, newest), (old.cargue.seq, new.cargue.seq));
            }
            other => panic!("expected Superseded, got {other:?}"),
        }
        assert_eq!(
            store
                .newest("Manual_1_abc")
                .expect("newest")
                .map(|c| c.sha256),
            Some(sha('b'))
        );
        assert_eq!(
            store
                .get("Manual_1_abc", &sha('a'))
                .expect("get")
                .map(|c| c.seq),
            Some(old.cargue.seq)
        );
        assert!(store.newest("other").expect("none").is_none());
    }

    #[test]
    fn the_same_bytes_under_another_id_are_a_new_cargue() {
        let db = TestDb::new();
        let store = CarguesStore::new(db.pool());
        assert!(
            store
                .ingest(&row("Manual_1.csv", 'a'))
                .expect("one")
                .created
        );
        assert!(
            store
                .ingest(&row("Manual_2.csv", 'a'))
                .expect("two")
                .created
        );
    }

    #[test]
    fn invalid_ids_and_hashes_are_rejected_before_the_database() {
        let db = TestDb::new();
        let store = CarguesStore::new(db.pool());
        let mut bad_id = row("Manual_1.csv", 'a');
        bad_id.id = "a__b".into();
        assert!(matches!(
            store.ingest(&bad_id),
            Err(IngestError::Invalid(_))
        ));
        let mut bad_sha = row("Manual_1.csv", 'a');
        bad_sha.sha256 = "ABC".into();
        assert!(matches!(
            store.ingest(&bad_sha),
            Err(IngestError::Invalid(_))
        ));
    }

    #[test]
    fn recent_lists_only_the_newest_upload_of_each_load() {
        let db = TestDb::new();
        let store = CarguesStore::new(db.pool());
        store.ingest(&row("Manual_1.csv", 'a')).expect("a");
        store.ingest(&row("Manual_1.csv", 'b')).expect("b");
        store.ingest(&row("Manual_2.csv", 'c')).expect("c");
        let since = Utc::now() - chrono::Duration::hours(1);
        let list = store.recent(since, 10).expect("recent");
        assert_eq!(list.len(), 2);
        assert!(
            list.iter()
                .any(|c| c.id == "Manual_1" && c.sha256 == sha('b'))
        );
        assert!(
            !list
                .iter()
                .any(|c| c.id == "Manual_1" && c.sha256 == sha('a'))
        );
    }
}
