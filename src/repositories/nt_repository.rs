use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};

use crate::db::schema::nt_records;
use crate::models::nt_record::{NewNtRecord, NtRecord};

/// Rows per `INSERT` statement.
///
/// Each row binds 8 parameters and SQLite's legacy limit is 999 bound
/// parameters per statement, so at most 124 rows fit; 100 stays safely below.
const INSERT_BATCH_SIZE: usize = 100;

pub struct NtRepository<'a> {
    conn: &'a mut SqliteConnection,
}

impl<'a> NtRepository<'a> {
    pub fn new(conn: &'a mut SqliteConnection) -> Self {
        Self { conn }
    }

    /// Inserts all records in batches inside a single transaction: either every
    /// record is stored or none is. Returns the number of inserted rows.
    pub fn insert_many(&mut self, records: &[NewNtRecord]) -> Result<usize> {
        self.conn
            .transaction::<usize, diesel::result::Error, _>(|conn| {
                let mut inserted = 0;
                for batch in records.chunks(INSERT_BATCH_SIZE) {
                    inserted += diesel::insert_into(nt_records::table)
                        .values(batch)
                        .execute(conn)?;
                }
                Ok(inserted)
            })
            .context("No fue posible guardar los registros de la Nota Técnica")
    }

    /// Copies the Nota Técnica stored in the database at `nt_db_path` (records
    /// and their `nt_source` row) into this database, keeping ids and row
    /// numbers. Returns the number of records copied.
    ///
    /// The path is bound as a parameter, never interpolated into SQL. `ATTACH`
    /// cannot run inside a transaction, so the copy runs in its own
    /// transaction between `ATTACH` and `DETACH`; `DETACH` is attempted even
    /// when the copy fails.
    pub fn snapshot_from(&mut self, nt_db_path: &Path) -> Result<usize> {
        let path = nt_db_path.to_str().ok_or_else(|| {
            anyhow!(
                "Ruta de la Nota Técnica no válida: {}",
                nt_db_path.display()
            )
        })?;
        diesel::sql_query("ATTACH DATABASE ? AS nt_store")
            .bind::<Text, _>(path)
            .execute(self.conn)
            .with_context(|| {
                format!(
                    "No fue posible abrir la Nota Técnica: {}",
                    nt_db_path.display()
                )
            })?;
        let copied = self.copy_attached();
        let detached = diesel::sql_query("DETACH DATABASE nt_store").execute(self.conn);
        let copied = copied?;
        detached.context("No fue posible cerrar la Nota Técnica adjunta")?;
        Ok(copied)
    }

    fn copy_attached(&mut self) -> Result<usize> {
        self.conn.transaction::<usize, anyhow::Error, _>(|conn| {
            let expected = diesel::sql_query("SELECT COUNT(*) AS n FROM nt_store.nt_records")
                .get_result::<RowCount>(conn)?
                .n;
            let copied = diesel::sql_query(
                "INSERT INTO nt_records (id, row_number, cum, cod_generico, forma_farmaceutica, \
                 concentracion, unidad_medida, clasificacion, precio) \
                 SELECT id, row_number, cum, cod_generico, forma_farmaceutica, \
                 concentracion, unidad_medida, clasificacion, precio \
                 FROM nt_store.nt_records ORDER BY id",
            )
            .execute(conn)?;
            if i64::try_from(copied).ok() != Some(expected) {
                bail!("La copia de la Nota Técnica está incompleta: {copied} de {expected} registros");
            }
            let sources = diesel::sql_query(
                "INSERT INTO nt_source (version, source_file, imported_at, records, skipped) \
                 SELECT version, source_file, imported_at, records, skipped FROM nt_store.nt_source",
            )
            .execute(conn)?;
            if sources != 1 {
                bail!("La Nota Técnica no tiene un origen registrado");
            }
            Ok(copied)
        })
        .context("No fue posible copiar la Nota Técnica al cargue")
    }

    #[allow(dead_code)]
    pub fn count(&mut self) -> Result<i64> {
        nt_records::table
            .count()
            .get_result(self.conn)
            .context("No fue posible contar los registros de la Nota Técnica")
    }

    #[allow(dead_code)]
    pub fn find_all(&mut self) -> Result<Vec<NtRecord>> {
        nt_records::table
            .order(nt_records::id.asc())
            .select(NtRecord::as_select())
            .load(self.conn)
            .context("No fue posible leer los registros de la Nota Técnica")
    }
}

#[derive(QueryableByName)]
struct RowCount {
    #[diesel(sql_type = BigInt)]
    n: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connection::{establish, run_load_migrations, run_nt_store_migrations};
    use std::path::Path;

    fn migrated_memory_db() -> SqliteConnection {
        let mut conn = establish(Path::new(":memory:")).expect("open in-memory db");
        run_load_migrations(&mut conn).expect("run migrations");
        conn
    }

    fn record(row_number: i32) -> NewNtRecord {
        NewNtRecord {
            row_number,
            cum: format!("CUM{row_number}"),
            cod_generico: "G1".into(),
            forma_farmaceutica: "TABLETA".into(),
            concentracion: "500 MG".into(),
            unidad_medida: "UN".into(),
            clasificacion: "POS".into(),
            precio: "1234,50".into(),
        }
    }

    #[test]
    fn insert_many_stores_every_record_across_batches() {
        let mut conn = migrated_memory_db();
        let records: Vec<_> = (2..252).map(record).collect();
        assert!(records.len() > INSERT_BATCH_SIZE * 2);

        let mut repo = NtRepository::new(&mut conn);
        let inserted = repo.insert_many(&records).expect("insert");

        assert_eq!(inserted, 250);
        assert_eq!(repo.count().expect("count"), 250);
    }

    #[test]
    fn insert_many_preserves_values_and_row_numbers() {
        let mut conn = migrated_memory_db();
        let mut repo = NtRepository::new(&mut conn);
        repo.insert_many(&[record(184)]).expect("insert");

        let stored = repo.find_all().expect("find_all");

        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].row_number, 184);
        assert_eq!(stored[0].cum, "CUM184");
        assert_eq!(stored[0].precio, "1234,50");
    }

    #[test]
    fn insert_many_with_no_records_inserts_nothing() {
        let mut conn = migrated_memory_db();
        let mut repo = NtRepository::new(&mut conn);

        assert_eq!(repo.insert_many(&[]).expect("insert"), 0);
        assert_eq!(repo.count().expect("count"), 0);
    }

    #[test]
    fn insert_many_rolls_back_everything_when_a_later_batch_fails() {
        let mut conn = migrated_memory_db();
        diesel::sql_query(
            "CREATE TRIGGER reject_poison BEFORE INSERT ON nt_records \
             WHEN NEW.row_number = 999 BEGIN SELECT RAISE(ABORT, 'poison row'); END",
        )
        .execute(&mut conn)
        .expect("create trigger");
        // Row 999 lands in the third batch, after two batches were already inserted.
        let records: Vec<_> = (700..1000).map(record).collect();

        let mut repo = NtRepository::new(&mut conn);
        assert!(repo.insert_many(&records).is_err());

        assert_eq!(repo.count().expect("count"), 0);
    }

    fn nt_store_with_records(path: &Path, count: i32) {
        let mut conn = establish(path).expect("open store");
        run_nt_store_migrations(&mut conn).expect("migrate store");
        let records: Vec<_> = (2..2 + count).map(record).collect();
        NtRepository::new(&mut conn)
            .insert_many(&records)
            .expect("insert");
        crate::repositories::nt_source_repository::NtSourceRepository::new(&mut conn)
            .insert(&crate::models::nt_source::NewNtSource {
                version: "v1".into(),
                source_file: "nota.xlsx".into(),
                imported_at: "2026-10-20T17:00:00-05:00".into(),
                records: count,
                skipped: 0,
            })
            .expect("insert source");
    }

    #[test]
    fn snapshot_copies_records_ids_and_source() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = dir.path().join("nt.db");
        nt_store_with_records(&store, 250);
        let mut conn = migrated_memory_db();
        let mut repo = NtRepository::new(&mut conn);

        let copied = repo.snapshot_from(&store).expect("snapshot");

        assert_eq!(copied, 250);
        let rows = repo.find_all().expect("find_all");
        assert_eq!(rows.len(), 250);
        assert_eq!((rows[0].id, rows[0].row_number), (1, 2));
        assert_eq!(rows[249].row_number, 251);
        let source = crate::repositories::nt_source_repository::NtSourceRepository::new(&mut conn)
            .find()
            .expect("find")
            .expect("source row");
        assert_eq!((source.version.as_str(), source.records), ("v1", 250));
    }

    #[test]
    fn snapshot_binds_paths_with_spaces_and_quotes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let odd_dir = dir
            .path()
            .join("it's a \"odd\" dir; DROP TABLE nt_records;--");
        std::fs::create_dir(&odd_dir).expect("mkdir");
        let store = odd_dir.join("nt.db");
        nt_store_with_records(&store, 3);
        let mut conn = migrated_memory_db();
        let mut repo = NtRepository::new(&mut conn);

        assert_eq!(repo.snapshot_from(&store).expect("snapshot"), 3);
        assert_eq!(repo.count().expect("count"), 3);
    }

    #[test]
    fn failed_snapshot_detaches_and_leaves_no_partial_data() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A valid SQLite file without the Nota Técnica tables.
        let broken = dir.path().join("broken.db");
        drop(establish(&broken).expect("create empty db"));
        let good = dir.path().join("nt.db");
        nt_store_with_records(&good, 5);
        let mut conn = migrated_memory_db();
        let mut repo = NtRepository::new(&mut conn);

        assert!(repo.snapshot_from(&broken).is_err());
        assert_eq!(repo.count().expect("count"), 0);
        // Re-attaching under the same alias only works if DETACH ran.
        assert_eq!(repo.snapshot_from(&good).expect("snapshot"), 5);
    }
}
