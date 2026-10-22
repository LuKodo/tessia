use std::fs;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use diesel::SqliteConnection;
use thiserror::Error;

use crate::config::DataLayout;
use crate::db::connection;
use crate::load::model::Load;
use crate::models::nt_source::NtSource;
use crate::repositories::nt_repository::NtRepository;
use crate::repositories::nt_source_repository::NtSourceRepository;
use crate::timing::{StepTimings, new_timestamped_id, timings_section};

const LOAD_DB_FILE: &str = "load.db";

// User-facing labels of the timed steps, in the order they are reported.
const STEP_CHECK_NT: &str = "Validación de la Nota Técnica";
const STEP_CREATE_LOAD: &str = "Creación del cargue";
const STEP_COPY_NT: &str = "Copia de la Nota Técnica";
const STEP_TOTAL: &str = "Total";

#[derive(Debug, Error)]
enum LoadError {
    #[error(
        "No hay una Nota Técnica cargada. Ejecute primero: facturacion nt import <archivo.xlsx>"
    )]
    NoNotaTecnica,
    #[error("El cargue {0} no tiene registrada la Nota Técnica usada")]
    MissingNtSource(String),
}

/// Result of a completed `load` command.
#[derive(Debug)]
pub struct LoadReport {
    pub load: Load,
    /// The Nota Técnica version copied into the load.
    pub nt_source: NtSource,
    pub timings: StepTimings,
}

/// Runs the `load` command end to end and prints the summary.
pub fn run() -> Result<()> {
    let report = create_cargue(&DataLayout::from_env())?;
    print_summary(&report);
    Ok(())
}

/// Creates a new load under the data root and snapshots the current Nota
/// Técnica into its own database.
///
/// Fails before creating anything when no Nota Técnica has been imported. If
/// the snapshot fails after the load directory was created, the directory is
/// left in place for inspection and the error is logged and returned.
pub fn create_cargue(layout: &DataLayout) -> Result<LoadReport> {
    let started = Instant::now();
    let mut timings = StepTimings::new();

    let nt_db = layout.nt_db();
    timings.time(STEP_CHECK_NT, || require_current_nt(&nt_db))?;
    let (load, mut conn) = timings.time(STEP_CREATE_LOAD, || create_load(&layout.loads_dir()))?;
    tracing::info!(load_id = %load.id, directory = %load.directory.display(), "load created");

    let snapshot = timings.time(STEP_COPY_NT, || snapshot_nt(&nt_db, &load, &mut conn));
    match snapshot {
        Ok(nt_source) => {
            timings.record(STEP_TOTAL, started.elapsed());
            Ok(LoadReport {
                load,
                nt_source,
                timings,
            })
        }
        Err(error) => {
            tracing::error!(
                load_id = %load.id,
                directory = %load.directory.display(),
                elapsed_ms = started.elapsed().as_millis() as u64,
                error = %format!("{error:#}"),
                "load failed; its directory was kept"
            );
            Err(error)
        }
    }
}

fn require_current_nt(nt_db: &Path) -> Result<(), LoadError> {
    if nt_db.is_file() {
        Ok(())
    } else {
        Err(LoadError::NoNotaTecnica)
    }
}

/// Copies the Nota Técnica into the load's database and returns its source row.
fn snapshot_nt(nt_db: &Path, load: &Load, conn: &mut SqliteConnection) -> Result<NtSource> {
    let copied = NtRepository::new(conn).snapshot_from(nt_db)?;
    tracing::debug!(copied, "Nota Técnica snapshot stored");
    NtSourceRepository::new(conn)
        .find()?
        .ok_or_else(|| anyhow!(LoadError::MissingNtSource(load.id.clone())))
}

/// Creates the directory and database of a new load, with migrations applied.
pub fn create_load(base_dir: &Path) -> Result<(Load, SqliteConnection)> {
    let id = new_timestamped_id();
    let directory = base_dir.join(&id);
    fs::create_dir_all(base_dir)
        .with_context(|| format!("No fue posible crear el directorio {}", base_dir.display()))?;
    // Not `create_dir_all`: an already existing load directory must be an error.
    fs::create_dir(&directory)
        .with_context(|| format!("No fue posible crear el cargue en {}", directory.display()))?;

    let database_path = directory.join(LOAD_DB_FILE);
    let mut conn = connection::establish(&database_path)?;
    connection::run_load_migrations(&mut conn)?;

    Ok((
        Load {
            id,
            directory,
            database_path,
        },
        conn,
    ))
}

fn print_summary(report: &LoadReport) {
    println!("Cargue creado: {}", report.load.id);
    println!();
    println!("Nota Técnica usada:");
    println!("  Versión: {}", report.nt_source.version);
    println!("  Archivo origen: {}", report.nt_source.source_file);
    println!("  Registros: {}", report.nt_source.records);
    println!();
    println!("Base de datos:");
    println!("  {}", report.load.database_path.display());
    for line in timings_section(&report.timings) {
        println!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NtImportConfig;
    use crate::nt_store::service::import_nota_tecnica;
    use crate::nt_store::test_support::write_sample_nt;
    use std::path::PathBuf;

    fn layout_in(dir: &tempfile::TempDir) -> DataLayout {
        DataLayout::new(dir.path().join("data"))
    }

    fn import_nt(layout: &DataLayout, dir: &tempfile::TempDir, count: usize, tag: &str) -> String {
        let source: PathBuf = dir.path().join(format!("nt_{tag}.xlsx"));
        write_sample_nt(&source, count, tag);
        import_nota_tecnica(&source, layout, &NtImportConfig::default())
            .expect("import")
            .version
    }

    fn load_counts(report: &LoadReport) -> (i64, String) {
        let mut conn = connection::establish(&report.load.database_path).expect("open load.db");
        let count = NtRepository::new(&mut conn).count().expect("count");
        let version = NtSourceRepository::new(&mut conn)
            .find()
            .expect("find")
            .expect("source")
            .version;
        (count, version)
    }

    #[test]
    fn create_load_builds_directory_and_migrated_database() {
        let base = tempfile::tempdir().expect("tempdir");

        let (load, mut conn) = create_load(base.path()).expect("create load");

        assert_eq!(load.directory, base.path().join(&load.id));
        assert_eq!(load.database_path, load.directory.join("load.db"));
        assert!(load.database_path.is_file());
        assert_eq!(NtRepository::new(&mut conn).count().expect("count"), 0);
    }

    #[test]
    fn each_load_gets_its_own_directory() {
        let base = tempfile::tempdir().expect("tempdir");

        let (first, _) = create_load(base.path()).expect("first");
        let (second, _) = create_load(base.path()).expect("second");

        assert_ne!(first.directory, second.directory);
    }

    #[test]
    fn load_without_a_nota_tecnica_fails_and_creates_no_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let layout = layout_in(&dir);

        let err = create_cargue(&layout).expect_err("must fail");

        assert_eq!(
            err.to_string(),
            "No hay una Nota Técnica cargada. Ejecute primero: facturacion nt import <archivo.xlsx>"
        );
        assert!(!layout.loads_dir().exists());
    }

    #[test]
    fn load_snapshots_the_current_nota_tecnica_without_copying_the_workbook() {
        let dir = tempfile::tempdir().expect("tempdir");
        let layout = layout_in(&dir);
        let version = import_nt(&layout, &dir, 7, "A");

        let report = create_cargue(&layout).expect("load");

        assert_eq!(load_counts(&report), (7, version.clone()));
        assert_eq!(report.nt_source.version, version);
        assert_eq!(report.nt_source.records, 7);
        assert_eq!(report.nt_source.source_file, "nt_A.xlsx");
        let entries: Vec<_> = fs::read_dir(&report.load.directory)
            .expect("read_dir")
            .map(|e| e.expect("entry").file_name())
            .collect();
        assert_eq!(entries, ["load.db"]);
    }

    #[test]
    fn loads_keep_their_own_snapshot_when_a_newer_nota_tecnica_is_imported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let layout = layout_in(&dir);
        let old_version = import_nt(&layout, &dir, 3, "A");
        let old_load = create_cargue(&layout).expect("old load");

        let new_version = import_nt(&layout, &dir, 5, "B");
        let new_load = create_cargue(&layout).expect("new load");

        assert_eq!(load_counts(&old_load), (3, old_version));
        assert_eq!(load_counts(&new_load), (5, new_version));
    }
}
