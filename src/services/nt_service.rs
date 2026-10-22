use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use diesel::SqliteConnection;

use crate::config::NtImportConfig;
use crate::importers::nt_importer::{NtImportResult, SkippedRow, import_nt};
use crate::repositories::nt_repository::NtRepository;
use crate::timing::measure;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NtImportSummary {
    /// Non-empty data rows read from the sheet (valid plus skipped), so
    /// `read == inserted + skipped.len()`. Fully empty rows are not counted.
    pub read: usize,
    pub inserted: usize,
    /// Rows left out because a required field is empty; they are not stored.
    pub skipped: Vec<SkippedRow>,
    /// Time spent reading and parsing the Excel file.
    pub read_elapsed: Duration,
    /// Time spent inserting the records into SQLite.
    pub insert_elapsed: Duration,
}

/// Reads the Nota Técnica workbook at `path`; also returns how long it took.
pub fn read(path: &Path, config: &NtImportConfig) -> Result<(NtImportResult, Duration)> {
    tracing::info!(path = %path.display(), "reading Nota Técnica");
    let (result, elapsed) = measure(|| import_nt(path, config));
    Ok((result?, elapsed))
}

/// Stores the records of a read Nota Técnica in the database behind `conn`.
pub fn store(
    result: NtImportResult,
    read_elapsed: Duration,
    conn: &mut SqliteConnection,
) -> Result<NtImportSummary> {
    let NtImportResult { records, skipped } = result;
    let read = records.len() + skipped.len();
    tracing::debug!(read, "Nota Técnica rows read");
    for row in &skipped {
        tracing::debug!(
            row_number = row.row_number,
            empty_fields = %row.empty_fields.join(", "),
            "row skipped: required fields are empty"
        );
    }

    let (inserted, insert_elapsed) = measure(|| NtRepository::new(conn).insert_many(&records));
    let inserted = inserted?;
    tracing::info!(
        read,
        inserted,
        skipped = skipped.len(),
        "Nota Técnica stored"
    );

    Ok(NtImportSummary {
        read,
        inserted,
        skipped,
        read_elapsed,
        insert_elapsed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connection::{establish, run_load_migrations};
    use crate::models::nt_record::NewNtRecord;

    fn record(row_number: i32) -> NewNtRecord {
        NewNtRecord {
            row_number,
            cum: format!("CUM{row_number}"),
            cod_generico: "G1".into(),
            forma_farmaceutica: "TABLETA".into(),
            concentracion: "500 MG".into(),
            unidad_medida: "UN".into(),
            clasificacion: "POS".into(),
            precio: "10".into(),
        }
    }

    #[test]
    fn read_equals_inserted_plus_skipped_and_skipped_rows_are_not_stored() {
        let mut conn = establish(Path::new(":memory:")).expect("open");
        run_load_migrations(&mut conn).expect("migrate");
        let result = NtImportResult {
            records: vec![record(2), record(3), record(5)],
            skipped: vec![SkippedRow {
                row_number: 4,
                empty_fields: vec!["PRECIO"],
            }],
        };

        let summary = store(result, Duration::ZERO, &mut conn).expect("store");

        assert_eq!(summary.read, 4);
        assert_eq!(summary.inserted, 3);
        assert_eq!(summary.skipped.len(), 1);
        assert_eq!(summary.read, summary.inserted + summary.skipped.len());
        let stored: Vec<i32> = NtRepository::new(&mut conn)
            .find_all()
            .expect("find_all")
            .iter()
            .map(|r| r.row_number)
            .collect();
        assert_eq!(stored, vec![2, 3, 5]);
    }
}
