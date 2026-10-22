//! Centralized configuration: where the data lives and how the Nota Técnica
//! workbook is read.

use std::env;
use std::path::PathBuf;

/// Environment variable that overrides the data root directory.
pub const DATA_DIR_ENV: &str = "FACTURACION_DATA_DIR";

const DEFAULT_DATA_DIR: &str = "data";
const LOADS_SUBDIR: &str = "loads";
const NT_SUBDIR: &str = "nt";
const HISTORY_SUBDIR: &str = "history";
const NT_DB_FILE: &str = "nt.db";
const NT_XLSX_FILE: &str = "nota_tecnica.xlsx";

/// On-disk layout under the data root:
///
/// ```text
/// <root>/nt/nt.db                                   current Nota Técnica
/// <root>/nt/nota_tecnica.xlsx                       its original workbook
/// <root>/nt/history/nt_<version>.db                 previous versions
/// <root>/nt/history/nota_tecnica_<version>.xlsx
/// <root>/loads/<load id>/load.db                    one directory per cargue
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataLayout {
    root: PathBuf,
}

impl DataLayout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Uses `FACTURACION_DATA_DIR` as the data root, or `data` (relative to the
    /// working directory) when it is unset or empty.
    pub fn from_env() -> Self {
        let root = env::var_os(DATA_DIR_ENV)
            .filter(|value| !value.is_empty())
            .map_or_else(|| PathBuf::from(DEFAULT_DATA_DIR), PathBuf::from);
        Self::new(root)
    }

    pub fn loads_dir(&self) -> PathBuf {
        self.root.join(LOADS_SUBDIR)
    }

    pub fn nt_dir(&self) -> PathBuf {
        self.root.join(NT_SUBDIR)
    }

    pub fn nt_db(&self) -> PathBuf {
        self.nt_dir().join(NT_DB_FILE)
    }

    pub fn nt_xlsx(&self) -> PathBuf {
        self.nt_dir().join(NT_XLSX_FILE)
    }

    pub fn nt_history_dir(&self) -> PathBuf {
        self.nt_dir().join(HISTORY_SUBDIR)
    }

    pub fn history_db(&self, version: &str) -> PathBuf {
        self.nt_history_dir().join(format!("nt_{version}.db"))
    }

    pub fn history_xlsx(&self, version: &str) -> PathBuf {
        self.nt_history_dir()
            .join(format!("nota_tecnica_{version}.xlsx"))
    }
}

/// Header text expected in the Nota Técnica for each persisted column.
///
/// Matching against the sheet header is trimmed and case-insensitive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NtColumnMapping {
    pub cum: String,
    pub cod_generico: String,
    pub forma_farmaceutica: String,
    pub concentracion: String,
    pub unidad_medida: String,
    pub clasificacion: String,
    pub precio: String,
}

/// Where and how to read the Nota Técnica workbook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NtImportConfig {
    /// Sheet name; `None` reads the first sheet of the workbook.
    pub sheet: Option<String>,
    /// 1-based Excel row that holds the column headers.
    pub header_row: u32,
    pub mapping: NtColumnMapping,
}

/// Defaults for the real Nota Técnica workbook ("Nota tecnica.xlsx").
///
/// The workbook has a hidden first sheet `NT` that is NOT the one wanted, so
/// the visible sheet `Hoja1` is named explicitly. Its header is on row 1; the
/// other columns (`Referencia`, `Notas`, `IDMUTUAL`, ...) are ignored.
///
/// `clasificacion` is read from `TIPO PRODUCTO` (confirmed by the user). Rows
/// with an empty `PRECIO CONTRATO` are legitimate in the source file; they are
/// still skipped and reported in the summary, not treated as an import error.
impl Default for NtImportConfig {
    fn default() -> Self {
        Self {
            sheet: Some("Hoja1".into()),
            header_row: 1,
            mapping: NtColumnMapping {
                cum: "MED-CUM".into(),
                cod_generico: "COD GEN".into(),
                forma_farmaceutica: "FORMA FARMACEUTICA RIPS".into(),
                concentracion: "CONCENTRACION MEDICAMENTO RIPS".into(),
                unidad_medida: "UNIDAD MEDIDA RIPS".into(),
                clasificacion: "TIPO PRODUCTO".into(),
                precio: "PRECIO CONTRATO".into(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn layout_places_current_history_and_loads_under_the_root() {
        let layout = DataLayout::new("root");

        assert_eq!(layout.loads_dir(), Path::new("root/loads"));
        assert_eq!(layout.nt_db(), Path::new("root/nt/nt.db"));
        assert_eq!(layout.nt_xlsx(), Path::new("root/nt/nota_tecnica.xlsx"));
        assert_eq!(layout.history_db("V"), Path::new("root/nt/history/nt_V.db"));
        assert_eq!(
            layout.history_xlsx("V"),
            Path::new("root/nt/history/nota_tecnica_V.xlsx")
        );
    }

    #[test]
    fn default_import_config_matches_the_real_nota_tecnica() {
        let config = NtImportConfig::default();

        assert_eq!(config.sheet.as_deref(), Some("Hoja1"));
        assert_eq!(config.header_row, 1);
        assert_eq!(config.mapping.cum, "MED-CUM");
        assert_eq!(config.mapping.cod_generico, "COD GEN");
        assert_eq!(config.mapping.forma_farmaceutica, "FORMA FARMACEUTICA RIPS");
        assert_eq!(
            config.mapping.concentracion,
            "CONCENTRACION MEDICAMENTO RIPS"
        );
        assert_eq!(config.mapping.unidad_medida, "UNIDAD MEDIDA RIPS");
        assert_eq!(config.mapping.clasificacion, "TIPO PRODUCTO");
        assert_eq!(config.mapping.precio, "PRECIO CONTRATO");
    }
}
