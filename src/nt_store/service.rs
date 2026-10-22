//! `nt import`: builds a new global Nota Técnica database, archives the
//! previous one under `history/` and makes the new one current.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use chrono::Local;
use diesel::SqliteConnection;
use thiserror::Error;

use crate::config::{DataLayout, NtImportConfig};
use crate::db::connection;
use crate::importers::nt_importer::{NtImportResult, SkippedRow};
use crate::models::nt_source::NewNtSource;
use crate::nt_store::model::NtImportReport;
use crate::repositories::nt_source_repository::NtSourceRepository;
use crate::services::nt_service::{self, NtImportSummary};
use crate::timing::{StepTimings, new_timestamped_id, timings_section};

/// Maximum number of skipped rows listed in the "Advertencias:" section.
const MAX_LISTED_SKIPPED_ROWS: usize = 20;

/// Temporary names, inside the Nota Técnica directory, of the version being built.
const STAGING_DB: &str = ".nt_new.db";
const STAGING_XLSX: &str = ".nota_tecnica_new.xlsx";

// User-facing labels of the timed steps, in the order they are reported.
const STEP_VALIDATE: &str = "Validación";
const STEP_READ_EXCEL: &str = "Lectura del Excel";
const STEP_CREATE_DB: &str = "Creación de la base";
const STEP_INSERT: &str = "Inserción en SQLite";
const STEP_COPY_FILE: &str = "Copia del archivo";
const STEP_ARCHIVE: &str = "Archivado de la versión anterior";
const STEP_TOTAL: &str = "Total";

#[derive(Debug, Error)]
enum NtFileError {
    #[error("La Nota Técnica no existe: {}", .0.display())]
    NotFound(PathBuf),
    #[error("El archivo debe ser .xlsx")]
    NotXlsx,
}

#[derive(Debug, Error)]
enum NtStoreError {
    #[error("La Nota Técnica no contiene registros válidos")]
    NoValidRecords,
    #[error("No se puede archivar la versión anterior: ya existe {}", .0.display())]
    HistoryFileExists(PathBuf),
}

/// Paths of the new version while it is being built.
struct Staging {
    db: PathBuf,
    xlsx: PathBuf,
}

impl Staging {
    fn new(layout: &DataLayout) -> Self {
        let dir = layout.nt_dir();
        Self {
            db: dir.join(STAGING_DB),
            xlsx: dir.join(STAGING_XLSX),
        }
    }

    /// Removes the staged files (and SQLite's journal), ignoring missing ones.
    fn clear(&self) -> Result<()> {
        let journal = PathBuf::from(format!("{}-journal", self.db.display()));
        for path in [&self.db, &journal, &self.xlsx] {
            match fs::remove_file(path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    return Err(error).with_context(|| {
                        format!(
                            "No fue posible eliminar el archivo temporal {}",
                            path.display()
                        )
                    });
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// Runs `nt import <file>` end to end and prints the summary.
pub fn run(file: &Path) -> Result<()> {
    let report = import_nota_tecnica(file, &DataLayout::from_env(), &NtImportConfig::default())?;
    print_summary(file, &report);
    Ok(())
}

/// Validates and reads the workbook first; only then builds the new Nota
/// Técnica, archives the previous one and swaps the new one in. Any failure
/// before the swap leaves the current Nota Técnica untouched.
pub fn import_nota_tecnica(
    file: &Path,
    layout: &DataLayout,
    config: &NtImportConfig,
) -> Result<NtImportReport> {
    let started = Instant::now();
    let mut timings = StepTimings::new();

    timings.time(STEP_VALIDATE, || validate_nt_path(file))?;
    let (result, read_elapsed) = nt_service::read(file, config)?;
    timings.record(STEP_READ_EXCEL, read_elapsed);
    if result.records.is_empty() {
        return Err(NtStoreError::NoValidRecords.into());
    }

    let version = new_timestamped_id();
    let staging = Staging::new(layout);
    let outcome = build_and_swap(
        file,
        layout,
        &staging,
        &version,
        (result, read_elapsed),
        &mut timings,
    );
    let (summary, archived_database) = match outcome {
        Ok(done) => done,
        Err(error) => {
            if let Err(cleanup) = staging.clear() {
                tracing::warn!(error = %format!("{cleanup:#}"), "could not remove staged files");
            }
            tracing::error!(
                version = %version,
                elapsed_ms = started.elapsed().as_millis() as u64,
                error = %format!("{error:#}"),
                "Nota Técnica import failed; the current one was kept"
            );
            return Err(error);
        }
    };
    timings.record(STEP_TOTAL, started.elapsed());
    tracing::info!(version = %version, "Nota Técnica imported");

    Ok(NtImportReport {
        version,
        database_path: layout.nt_db(),
        archived_database,
        summary,
        timings,
    })
}

fn build_and_swap(
    file: &Path,
    layout: &DataLayout,
    staging: &Staging,
    version: &str,
    (result, read_elapsed): (NtImportResult, std::time::Duration),
    timings: &mut StepTimings,
) -> Result<(NtImportSummary, Option<PathBuf>)> {
    let nt_dir = layout.nt_dir();
    fs::create_dir_all(&nt_dir)
        .with_context(|| format!("No fue posible crear el directorio {}", nt_dir.display()))?;
    staging.clear()?;

    let source = new_source(file, version, &result)?;
    let mut conn = timings.time(STEP_CREATE_DB, || create_store_db(&staging.db))?;
    let summary = nt_service::store(result, read_elapsed, &mut conn)?;
    timings.record(STEP_INSERT, summary.insert_elapsed);
    NtSourceRepository::new(&mut conn).insert(&source)?;
    // The connection must be closed before any rename (required on Windows).
    drop(conn);

    timings
        .time(STEP_COPY_FILE, || fs::copy(file, &staging.xlsx))
        .with_context(|| {
            format!(
                "No fue posible copiar la Nota Técnica a {}",
                staging.xlsx.display()
            )
        })?;

    let archived = timings.time(STEP_ARCHIVE, || swap_in(layout, staging))?;
    Ok((summary, archived))
}

fn create_store_db(path: &Path) -> Result<SqliteConnection> {
    let mut conn = connection::establish(path)?;
    connection::run_nt_store_migrations(&mut conn)?;
    Ok(conn)
}

fn new_source(file: &Path, version: &str, result: &NtImportResult) -> Result<NewNtSource> {
    let count =
        |value: usize| i32::try_from(value).context("La Nota Técnica tiene demasiados registros");
    Ok(NewNtSource {
        version: version.to_owned(),
        source_file: file.file_name().map_or_else(
            || file.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        ),
        imported_at: Local::now().to_rfc3339(),
        records: count(result.records.len())?,
        skipped: count(result.skipped.len())?,
    })
}

fn validate_nt_path(path: &Path) -> Result<(), NtFileError> {
    if !path.exists() {
        return Err(NtFileError::NotFound(path.to_path_buf()));
    }
    let is_xlsx = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("xlsx"));
    if !is_xlsx {
        return Err(NtFileError::NotXlsx);
    }
    Ok(())
}

/// Archives the current Nota Técnica (if any) into `history/`, named with its
/// own version, and moves the staged one into place. Returns the archived
/// database path. If a rename fails, the ones already done are undone.
fn swap_in(layout: &DataLayout, staging: &Staging) -> Result<Option<PathBuf>> {
    let (current_db, current_xlsx) = (layout.nt_db(), layout.nt_xlsx());
    let mut moves = Vec::new();
    let mut archived_database = None;

    if current_db.exists() || current_xlsx.exists() {
        let old_version = previous_version(&current_db);
        let history_db = layout.history_db(&old_version);
        let history_xlsx = layout.history_xlsx(&old_version);
        for (from, to) in [(&current_db, &history_db), (&current_xlsx, &history_xlsx)] {
            if !from.exists() {
                continue;
            }
            if to.exists() {
                return Err(NtStoreError::HistoryFileExists(to.clone()).into());
            }
            moves.push((from.clone(), to.clone()));
        }
        if current_db.exists() {
            archived_database = Some(history_db);
        }
        let history_dir = layout.nt_history_dir();
        fs::create_dir_all(&history_dir).with_context(|| {
            format!(
                "No fue posible crear el directorio {}",
                history_dir.display()
            )
        })?;
    }
    moves.push((staging.db.clone(), current_db));
    moves.push((staging.xlsx.clone(), current_xlsx));

    rename_all(&moves)?;
    Ok(archived_database)
}

/// Version recorded in the current database, or a fresh unique id when it
/// cannot be read (or is not safe to use in a file name).
fn previous_version(current_db: &Path) -> String {
    let stored = connection::establish(current_db)
        .and_then(|mut conn| NtSourceRepository::new(&mut conn).find())
        .map(|source| source.map(|s| s.version));
    match stored {
        Ok(Some(version)) if is_safe_file_part(&version) => version,
        other => {
            let reason = other.err().map(|error| format!("{error:#}"));
            tracing::warn!(
                ?reason,
                "previous version unknown; archiving under a new id"
            );
            new_timestamped_id()
        }
    }
}

fn is_safe_file_part(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Performs the renames in order; if one fails, undoes the completed ones
/// (best effort) and returns the error.
fn rename_all(moves: &[(PathBuf, PathBuf)]) -> Result<()> {
    for (done, (from, to)) in moves.iter().enumerate() {
        if let Err(error) = fs::rename(from, to) {
            undo_renames(&moves[..done]);
            return Err(error).with_context(|| {
                format!("No fue posible mover {} a {}", from.display(), to.display())
            });
        }
    }
    Ok(())
}

fn undo_renames(done: &[(PathBuf, PathBuf)]) {
    for (from, to) in done.iter().rev() {
        if let Err(error) = fs::rename(to, from) {
            tracing::error!(
                from = %to.display(),
                to = %from.display(),
                %error,
                "could not restore a file after a failed swap"
            );
        }
    }
}

fn print_summary(file: &Path, report: &NtImportReport) {
    let summary = &report.summary;
    println!("Nota Técnica importada: {}", report.version);
    println!();
    println!("Nota Técnica:");
    println!("  Archivo: {}", file.display());
    println!("  Registros leídos: {}", summary.read);
    println!("  Registros insertados: {}", summary.inserted);
    println!("  Registros omitidos: {}", summary.skipped.len());
    for line in warnings_section(&summary.skipped) {
        println!("{line}");
    }
    println!();
    println!("Base de datos:");
    println!("  {}", report.database_path.display());
    if let Some(archived) = &report.archived_database {
        println!("Versión anterior archivada en:");
        println!("  {}", archived.display());
    }
    for line in timings_section(&report.timings) {
        println!("{line}");
    }
}

/// Lines of the "Advertencias:" section, preceded by a blank line; empty when
/// no row was skipped. Lists at most [`MAX_LISTED_SKIPPED_ROWS`] rows.
fn warnings_section(skipped: &[SkippedRow]) -> Vec<String> {
    if skipped.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![
        String::new(),
        "Advertencias:".to_owned(),
        match skipped.len() {
            1 => "  Se omitió 1 fila con campos vacíos (no se guardó):".to_owned(),
            count => format!("  Se omitieron {count} filas con campos vacíos (no se guardaron):"),
        },
    ];
    lines.extend(
        skipped
            .iter()
            .take(MAX_LISTED_SKIPPED_ROWS)
            .map(|row| format!("  {row}")),
    );
    if skipped.len() > MAX_LISTED_SKIPPED_ROWS {
        lines.push(format!(
            "  ... y {} más",
            skipped.len() - MAX_LISTED_SKIPPED_ROWS
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connection::establish;
    use crate::nt_store::test_support::{REAL_HEADER, sample_rows, write_sample_nt, write_xlsx};
    use crate::repositories::nt_repository::NtRepository;

    fn layout_in(dir: &tempfile::TempDir) -> DataLayout {
        DataLayout::new(dir.path().join("data"))
    }

    fn import(layout: &DataLayout, source: &Path) -> Result<NtImportReport> {
        import_nota_tecnica(source, layout, &NtImportConfig::default())
    }

    fn source_with(dir: &tempfile::TempDir, name: &str, count: usize, tag: &str) -> PathBuf {
        let path = dir.path().join(name);
        write_sample_nt(&path, count, tag);
        path
    }

    fn current_version(layout: &DataLayout) -> String {
        let mut conn = establish(&layout.nt_db()).expect("open nt.db");
        NtSourceRepository::new(&mut conn)
            .find()
            .expect("find")
            .expect("source row")
            .version
    }

    fn file_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .expect("read_dir")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn validate_rejects_missing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("nope.xlsx");

        let err = validate_nt_path(&missing).expect_err("must fail");

        assert_eq!(
            err.to_string(),
            format!("La Nota Técnica no existe: {}", missing.display())
        );
    }

    #[test]
    fn validate_rejects_non_xlsx_extension() {
        let dir = tempfile::tempdir().expect("tempdir");
        let csv = dir.path().join("nt.csv");
        fs::write(&csv, "a,b").expect("write");

        let err = validate_nt_path(&csv).expect_err("must fail");

        assert_eq!(err.to_string(), "El archivo debe ser .xlsx");
    }

    #[test]
    fn validate_accepts_xlsx_extension_case_insensitively() {
        let dir = tempfile::tempdir().expect("tempdir");
        for name in ["a.xlsx", "b.XLSX", "c.XlSx"] {
            let file = dir.path().join(name);
            fs::write(&file, "x").expect("write");
            assert!(validate_nt_path(&file).is_ok(), "{name} should be accepted");
        }
    }

    #[test]
    fn first_import_creates_the_current_files_and_no_history() {
        let dir = tempfile::tempdir().expect("tempdir");
        let layout = layout_in(&dir);
        let source = source_with(&dir, "Nota tecnica.xlsx", 3, "A");

        let report = import(&layout, &source).expect("import");

        assert_eq!(report.summary.inserted, 3);
        assert_eq!(report.archived_database, None);
        assert_eq!(file_names(&layout.nt_dir()), ["nota_tecnica.xlsx", "nt.db"]);
        assert!(!layout.nt_history_dir().exists());
        assert_eq!(current_version(&layout), report.version);
        let mut conn = establish(&layout.nt_db()).expect("open");
        let source_row = NtSourceRepository::new(&mut conn)
            .find()
            .expect("find")
            .expect("row");
        assert_eq!(source_row.source_file, "Nota tecnica.xlsx");
        assert_eq!((source_row.records, source_row.skipped), (3, 0));
        assert!(chrono::DateTime::parse_from_rfc3339(&source_row.imported_at).is_ok());
        assert_eq!(NtRepository::new(&mut conn).count().expect("count"), 3);
    }

    #[test]
    fn second_import_archives_the_first_under_its_own_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        let layout = layout_in(&dir);
        let first = import(&layout, &source_with(&dir, "a.xlsx", 3, "A")).expect("first");
        let second = import(&layout, &source_with(&dir, "b.xlsx", 5, "B")).expect("second");

        assert_ne!(first.version, second.version);
        assert_eq!(current_version(&layout), second.version);
        assert_eq!(
            second.archived_database,
            Some(layout.history_db(&first.version))
        );
        assert_eq!(
            file_names(&layout.nt_history_dir()),
            [
                format!("nota_tecnica_{}.xlsx", first.version),
                format!("nt_{}.db", first.version),
            ]
        );
        assert_eq!(
            file_names(&layout.nt_dir()),
            ["history", "nota_tecnica.xlsx", "nt.db"]
        );
        let mut old = establish(&layout.history_db(&first.version)).expect("open old");
        assert_eq!(NtRepository::new(&mut old).count().expect("count"), 3);
        let mut current = establish(&layout.nt_db()).expect("open current");
        assert_eq!(NtRepository::new(&mut current).count().expect("count"), 5);
    }

    #[test]
    fn failed_imports_leave_the_current_nt_byte_identical() {
        let dir = tempfile::tempdir().expect("tempdir");
        let layout = layout_in(&dir);
        import(&layout, &source_with(&dir, "a.xlsx", 3, "A")).expect("first");
        let db_before = fs::read(layout.nt_db()).expect("read db");
        let xlsx_before = fs::read(layout.nt_xlsx()).expect("read xlsx");

        // Missing column.
        let mut rows = sample_rows(2, "B");
        for row in &mut rows {
            row.remove(0);
        }
        let missing_column = dir.path().join("missing.xlsx");
        write_xlsx(&missing_column, &rows);
        // Rows exist but every one has an empty required field.
        let mut rows = sample_rows(2, "C");
        for row in rows.iter_mut().skip(1) {
            row[9].clear();
        }
        let no_valid = dir.path().join("novalid.xlsx");
        write_xlsx(&no_valid, &rows);

        let err = import(&layout, &missing_column).expect_err("missing column");
        assert_eq!(err.to_string(), "No se encontró la columna MED-CUM");
        let err = import(&layout, &no_valid).expect_err("no valid records");
        assert_eq!(
            err.to_string(),
            "La Nota Técnica no contiene registros válidos"
        );
        assert!(import(&layout, &dir.path().join("nope.xlsx")).is_err());

        assert_eq!(fs::read(layout.nt_db()).expect("db"), db_before);
        assert_eq!(fs::read(layout.nt_xlsx()).expect("xlsx"), xlsx_before);
        assert!(!layout.nt_history_dir().exists());
        assert_eq!(file_names(&layout.nt_dir()), ["nota_tecnica.xlsx", "nt.db"]);
    }

    #[test]
    fn a_failed_first_import_creates_nothing_under_the_data_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let layout = layout_in(&dir);
        let no_valid = dir.path().join("novalid.xlsx");
        write_xlsx(
            &no_valid,
            &[REAL_HEADER.iter().map(|h| (*h).to_owned()).collect()],
        );

        assert!(import(&layout, &no_valid).is_err());

        assert!(!layout.nt_dir().exists());
    }

    #[test]
    fn leftovers_of_a_crashed_import_are_removed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let layout = layout_in(&dir);
        fs::create_dir_all(layout.nt_dir()).expect("mkdir");
        fs::write(layout.nt_dir().join(STAGING_DB), "junk").expect("write");
        fs::write(layout.nt_dir().join(STAGING_XLSX), "junk").expect("write");

        import(&layout, &source_with(&dir, "a.xlsx", 2, "A")).expect("import");

        assert_eq!(file_names(&layout.nt_dir()), ["nota_tecnica.xlsx", "nt.db"]);
    }

    #[test]
    fn unreadable_previous_database_is_archived_under_a_fresh_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let layout = layout_in(&dir);
        fs::create_dir_all(layout.nt_dir()).expect("mkdir");
        fs::write(layout.nt_db(), "not a sqlite file").expect("write");

        let report = import(&layout, &source_with(&dir, "a.xlsx", 2, "A")).expect("import");

        let archived = report.archived_database.expect("archived");
        assert!(archived.starts_with(layout.nt_history_dir()));
        assert_eq!(fs::read(&archived).expect("read"), b"not a sqlite file");
        assert_eq!(current_version(&layout), report.version);
    }

    #[test]
    fn rename_all_undoes_completed_renames_when_one_fails() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (a, b, c, d) = (
            dir.path().join("a"),
            dir.path().join("b"),
            dir.path().join("missing"),
            dir.path().join("d"),
        );
        fs::write(&a, "A").expect("write");

        let result = rename_all(&[(a.clone(), b.clone()), (c, d)]);

        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&a).expect("restored"), "A");
        assert!(!b.exists());
    }

    #[test]
    fn existing_history_file_blocks_the_archive_without_moving_anything() {
        let dir = tempfile::tempdir().expect("tempdir");
        let layout = layout_in(&dir);
        let first = import(&layout, &source_with(&dir, "a.xlsx", 2, "A")).expect("first");
        fs::create_dir_all(layout.nt_history_dir()).expect("mkdir");
        fs::write(layout.history_xlsx(&first.version), "taken").expect("write");
        let db_before = fs::read(layout.nt_db()).expect("read");

        let err = import(&layout, &source_with(&dir, "b.xlsx", 2, "B")).expect_err("must fail");

        assert!(
            err.to_string()
                .starts_with("No se puede archivar la versión anterior")
        );
        assert_eq!(fs::read(layout.nt_db()).expect("read"), db_before);
        assert!(!layout.history_db(&first.version).exists());
        assert_eq!(
            file_names(&layout.nt_dir()),
            ["history", "nota_tecnica.xlsx", "nt.db"]
        );
    }

    #[test]
    fn unsafe_versions_are_not_used_in_file_names() {
        assert!(is_safe_file_part("20261020_170000_ab12cd34"));
        assert!(!is_safe_file_part("../evil"));
        assert!(!is_safe_file_part(""));
    }

    fn skipped_rows(count: usize) -> Vec<SkippedRow> {
        (0..count)
            .map(|n| SkippedRow {
                row_number: n as i32 + 2,
                empty_fields: vec!["PRECIO"],
            })
            .collect()
    }

    #[test]
    fn warnings_section_is_empty_without_skipped_rows() {
        assert!(warnings_section(&[]).is_empty());
    }

    #[test]
    fn warnings_section_uses_singular_for_one_row() {
        let lines = warnings_section(&skipped_rows(1));

        assert_eq!(
            lines[2],
            "  Se omitió 1 fila con campos vacíos (no se guardó):"
        );
    }

    #[test]
    fn warnings_section_lists_every_row_below_the_cap() {
        let lines = warnings_section(&skipped_rows(2));

        assert_eq!(
            lines,
            vec![
                "",
                "Advertencias:",
                "  Se omitieron 2 filas con campos vacíos (no se guardaron):",
                "  Fila 2: campo PRECIO vacío",
                "  Fila 3: campo PRECIO vacío",
            ]
        );
    }

    #[test]
    fn warnings_section_lists_exactly_the_cap_without_a_remainder_line() {
        let lines = warnings_section(&skipped_rows(MAX_LISTED_SKIPPED_ROWS));

        assert_eq!(lines.len(), 3 + MAX_LISTED_SKIPPED_ROWS);
        assert!(!lines.last().expect("last").contains("más"));
    }

    #[test]
    fn warnings_section_caps_the_list_and_counts_the_rest() {
        let lines = warnings_section(&skipped_rows(25));

        assert_eq!(lines.len(), 3 + MAX_LISTED_SKIPPED_ROWS + 1);
        assert_eq!(
            lines[2],
            "  Se omitieron 25 filas con campos vacíos (no se guardaron):"
        );
        assert_eq!(lines.last().map(String::as_str), Some("  ... y 5 más"));
    }
}
