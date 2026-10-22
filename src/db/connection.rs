use std::path::Path;

use anyhow::{Context, Result, anyhow};
use diesel::{Connection, SqliteConnection};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};

/// Schema of the global Nota Técnica database (`<data root>/nt/nt.db`).
const NT_STORE_MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/nt_store");
/// Schema of each cargue's own database (`<data root>/loads/<id>/load.db`).
const LOAD_MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/load");

/// Opens (creating it if needed) the SQLite database at `path`.
pub fn establish(path: &Path) -> Result<SqliteConnection> {
    let url = path
        .to_str()
        .ok_or_else(|| anyhow!("Ruta de base de datos no válida: {}", path.display()))?;
    SqliteConnection::establish(url)
        .with_context(|| format!("No fue posible abrir la base de datos: {}", path.display()))
}

/// Applies the pending migrations of the global Nota Técnica database.
pub fn run_nt_store_migrations(conn: &mut SqliteConnection) -> Result<()> {
    run_migrations(conn, NT_STORE_MIGRATIONS)
}

/// Applies the pending migrations of a cargue database.
pub fn run_load_migrations(conn: &mut SqliteConnection) -> Result<()> {
    run_migrations(conn, LOAD_MIGRATIONS)
}

fn run_migrations(conn: &mut SqliteConnection, migrations: EmbeddedMigrations) -> Result<()> {
    conn.run_pending_migrations(migrations)
        .map_err(|e| anyhow!("No fue posible ejecutar las migraciones: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use diesel::RunQueryDsl;
    use diesel::sql_types::Text;

    #[derive(diesel::QueryableByName)]
    struct Name {
        #[diesel(sql_type = Text)]
        name: String,
    }

    fn schema_objects(conn: &mut SqliteConnection, kind: &str) -> Vec<String> {
        let rows: Vec<Name> = diesel::sql_query(
            "SELECT name FROM sqlite_master WHERE type = ? AND name NOT LIKE 'sqlite_%' \
             AND name NOT LIKE '\\_\\_diesel%' ESCAPE '\\' ORDER BY name",
        )
        .bind::<Text, _>(kind)
        .load(conn)
        .expect("query sqlite_master");
        rows.into_iter().map(|row| row.name).collect()
    }

    fn memory() -> SqliteConnection {
        establish(Path::new(":memory:")).expect("open")
    }

    #[test]
    fn nt_store_migrations_create_records_source_and_cum_index() {
        let mut conn = memory();
        run_nt_store_migrations(&mut conn).expect("migrate");

        assert_eq!(
            schema_objects(&mut conn, "table"),
            ["nt_records", "nt_source"]
        );
        assert_eq!(schema_objects(&mut conn, "index"), ["idx_nt_records_cum"]);
    }

    #[test]
    fn load_migrations_create_records_source_and_cum_index() {
        let mut conn = memory();
        run_load_migrations(&mut conn).expect("migrate");

        assert_eq!(
            schema_objects(&mut conn, "table"),
            ["nt_records", "nt_source"]
        );
        assert_eq!(schema_objects(&mut conn, "index"), ["idx_nt_records_cum"]);
    }
}
