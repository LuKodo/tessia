//! The "archived" marker of filed artifacts (Go moved a confirmed file into
//! `Radicado/`; the bucket cannot rename, so this table says it was filed).

use std::collections::HashMap;

use diesel::prelude::*;

use crate::db::{DbPool, DbResult};

diesel::table! {
    radicacion_archivo (cargue_id, rel_path) {
        cargue_id -> Text,
        rel_path -> Text,
        sha256 -> Text,
        archived_at -> Timestamptz,
    }
}

use radicacion_archivo as t;

#[derive(Clone)]
pub struct ArchiveStore {
    pool: DbPool,
}

impl ArchiveStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Marks `(cargue_id, rel_path)` with content `sha256` as filed.
    pub fn mark(&self, cargue_id: &str, rel_path: &str, sha256: &str) -> DbResult<()> {
        self.pool.run("mark radicacion archive", |conn| {
            diesel::insert_into(t::table)
                .values((
                    t::cargue_id.eq(cargue_id),
                    t::rel_path.eq(rel_path),
                    t::sha256.eq(sha256),
                ))
                .on_conflict((t::cargue_id, t::rel_path))
                .do_update()
                .set((t::sha256.eq(sha256), t::archived_at.eq(diesel::dsl::now)))
                .execute(conn)?;
            Ok(())
        })
    }

    /// `rel_path -> sha256` of every filed artifact of the cargue.
    pub fn list(&self, cargue_id: &str) -> DbResult<HashMap<String, String>> {
        self.pool.run("list radicacion archive", |conn| {
            let rows: Vec<(String, String)> = t::table
                .filter(t::cargue_id.eq(cargue_id))
                .select((t::rel_path, t::sha256))
                .load(conn)?;
            Ok(rows.into_iter().collect())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::TestDb;

    #[test]
    fn marks_are_listed_per_cargue_and_replaced_on_remark() {
        let db = TestDb::new();
        let store = ArchiveStore::new(db.pool());
        store
            .mark("C1", "XML/FEV1_XML_PDF.zip", "aaa")
            .expect("mark");
        store.mark("C1", "FEV1.txt", "bbb").expect("mark");
        store.mark("C2", "FEV1.txt", "ccc").expect("mark");
        store.mark("C1", "FEV1.txt", "ddd").expect("remark");
        let listed = store.list("C1").expect("list");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed["FEV1.txt"], "ddd");
        assert_eq!(listed["XML/FEV1_XML_PDF.zip"], "aaa");
        assert_eq!(store.list("C2").expect("list")["FEV1.txt"], "ccc");
        assert!(store.list("none").expect("list").is_empty());
    }
}
