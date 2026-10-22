use anyhow::{Context, Result};
use diesel::prelude::*;

use crate::db::schema::nt_source;
use crate::models::nt_source::{NewNtSource, NtSource};

pub struct NtSourceRepository<'a> {
    conn: &'a mut SqliteConnection,
}

impl<'a> NtSourceRepository<'a> {
    pub fn new(conn: &'a mut SqliteConnection) -> Self {
        Self { conn }
    }

    pub fn insert(&mut self, source: &NewNtSource) -> Result<()> {
        diesel::insert_into(nt_source::table)
            .values(source)
            .execute(self.conn)
            .context("No fue posible guardar el origen de la Nota Técnica")?;
        Ok(())
    }

    /// The single source row, if the database has one.
    pub fn find(&mut self) -> Result<Option<NtSource>> {
        nt_source::table
            .select(NtSource::as_select())
            .first(self.conn)
            .optional()
            .context("No fue posible leer el origen de la Nota Técnica")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connection::{establish, run_nt_store_migrations};
    use std::path::Path;

    fn source(version: &str) -> NewNtSource {
        NewNtSource {
            version: version.into(),
            source_file: "nota.xlsx".into(),
            imported_at: "2026-10-20T17:00:00-05:00".into(),
            records: 10,
            skipped: 2,
        }
    }

    #[test]
    fn find_returns_none_then_the_inserted_row() {
        let mut conn = establish(Path::new(":memory:")).expect("open");
        run_nt_store_migrations(&mut conn).expect("migrate");
        let mut repo = NtSourceRepository::new(&mut conn);
        assert_eq!(repo.find().expect("find"), None);

        repo.insert(&source("v1")).expect("insert");

        let found = repo.find().expect("find").expect("row");
        assert_eq!(found.version, "v1");
        assert_eq!((found.records, found.skipped), (10, 2));
    }

    #[test]
    fn a_duplicate_version_is_rejected() {
        let mut conn = establish(Path::new(":memory:")).expect("open");
        run_nt_store_migrations(&mut conn).expect("migrate");
        let mut repo = NtSourceRepository::new(&mut conn);
        repo.insert(&source("v1")).expect("insert");

        assert!(repo.insert(&source("v1")).is_err());
    }
}
