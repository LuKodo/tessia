//! `artefactos`: the index of the objects stored in the bucket per cargue.
//!
//! The bucket key is derived from `(cargue_id, rel_path)` when publishing and
//! never parsed back; this table is the source of truth for what exists, with
//! the size and SHA-256 used to verify downloads. Usually accessed through
//! [`crate::storage::Artifacts`].

mod schema;

use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::upsert::excluded;

use crate::db::{DbPool, DbResult};
use schema::artefactos;

/// An indexed object.
#[derive(Debug, Clone, PartialEq, Eq, Queryable, Selectable, Insertable)]
#[diesel(table_name = artefactos)]
pub struct Artefacto {
    pub cargue_id: String,
    /// Path inside the cargue workdir, `/`-separated (`XML/FEV1.xml`).
    pub rel_path: String,
    /// Bucket object name.
    pub key: String,
    pub size: i64,
    pub sha256: String,
    pub content_type: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = artefactos)]
struct NewArtefacto<'a> {
    cargue_id: &'a str,
    rel_path: &'a str,
    key: &'a str,
    size: i64,
    sha256: &'a str,
    content_type: &'a str,
}

#[derive(Clone)]
pub struct ArtefactosStore {
    pool: DbPool,
}

impl ArtefactosStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Inserts or replaces the index row of `(cargue_id, rel_path)`.
    pub fn upsert(
        &self,
        cargue_id: &str,
        rel_path: &str,
        key: &str,
        size: i64,
        sha256: &str,
        content_type: &str,
    ) -> DbResult<()> {
        let row = NewArtefacto {
            cargue_id,
            rel_path,
            key,
            size,
            sha256,
            content_type,
        };
        self.pool.run("upsert artefacto", |conn| {
            diesel::insert_into(artefactos::table)
                .values(&row)
                .on_conflict((artefactos::cargue_id, artefactos::rel_path))
                .do_update()
                .set((
                    artefactos::key.eq(excluded(artefactos::key)),
                    artefactos::size.eq(excluded(artefactos::size)),
                    artefactos::sha256.eq(excluded(artefactos::sha256)),
                    artefactos::content_type.eq(excluded(artefactos::content_type)),
                    artefactos::created_at.eq(diesel::dsl::now),
                ))
                .execute(conn)?;
            Ok(())
        })
    }

    pub fn get(&self, cargue_id: &str, rel_path: &str) -> DbResult<Option<Artefacto>> {
        self.pool.run("get artefacto", |conn| {
            Ok(artefactos::table
                .find((cargue_id, rel_path))
                .select(Artefacto::as_select())
                .first(conn)
                .optional()?)
        })
    }

    /// Every object of the cargue, ordered by relative path.
    pub fn list(&self, cargue_id: &str) -> DbResult<Vec<Artefacto>> {
        self.pool.run("list artefactos", |conn| {
            Ok(artefactos::table
                .filter(artefactos::cargue_id.eq(cargue_id))
                .order(artefactos::rel_path.asc())
                .select(Artefacto::as_select())
                .load(conn)?)
        })
    }

    /// Removes the index row; true when it existed.
    pub fn delete(&self, cargue_id: &str, rel_path: &str) -> DbResult<bool> {
        self.pool.run("delete artefacto", |conn| {
            Ok(diesel::delete(artefactos::table.find((cargue_id, rel_path))).execute(conn)? > 0)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::TestDb;

    #[test]
    fn upsert_replaces_and_list_is_ordered() {
        let db = TestDb::new();
        let store = ArtefactosStore::new(db.pool());
        let (a, b) = ("a".repeat(64), "b".repeat(64));
        store
            .upsert(
                "C1",
                "XML/B.xml",
                "C1__XML__B.xml",
                10,
                &a,
                "application/xml",
            )
            .expect("b");
        store
            .upsert(
                "C1",
                "CRC/A.zip",
                "C1__CRC__A.zip",
                5,
                &a,
                "application/zip",
            )
            .expect("a");
        store
            .upsert(
                "C1",
                "CRC/A.zip",
                "C1__CRC__A.zip",
                7,
                &b,
                "application/zip",
            )
            .expect("replace");
        store
            .upsert("C2", "x", "C2__x", 1, &a, "text/plain")
            .expect("other cargue");
        let list = store.list("C1").expect("list");
        let paths: Vec<&str> = list.iter().map(|r| r.rel_path.as_str()).collect();
        assert_eq!(paths, ["CRC/A.zip", "XML/B.xml"]);
        assert_eq!((list[0].size, list[0].sha256.as_str()), (7, b.as_str()));
        assert!(store.get("C1", "nope").expect("get").is_none());
        assert!(store.delete("C1", "XML/B.xml").expect("delete"));
        assert!(!store.delete("C1", "XML/B.xml").expect("delete again"));
    }

    #[test]
    fn keys_are_unique_across_cargues() {
        let db = TestDb::new();
        let store = ArtefactosStore::new(db.pool());
        let a = "a".repeat(64);
        store
            .upsert("C1", "x", "same-key", 1, &a, "text/plain")
            .expect("first");
        assert!(
            store
                .upsert("C2", "y", "same-key", 1, &a, "text/plain")
                .is_err()
        );
    }
}
