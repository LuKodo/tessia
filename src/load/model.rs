use std::path::PathBuf;

/// An independent load: its own directory and its own SQLite database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Load {
    pub id: String,
    pub directory: PathBuf,
    pub database_path: PathBuf,
}
