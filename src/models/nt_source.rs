use diesel::prelude::*;

use crate::db::schema::nt_source;

/// Where a Nota Técnica version came from. Single row per database: in
/// `nt.db` it describes that Nota Técnica; in a `load.db` it is a copy made
/// when the cargue took its snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Queryable, Selectable)]
#[diesel(table_name = nt_source, check_for_backend(diesel::sqlite::Sqlite))]
pub struct NtSource {
    pub version: String,
    /// File name only, not the full path.
    pub source_file: String,
    /// RFC 3339 timestamp.
    pub imported_at: String,
    /// Valid records stored.
    pub records: i32,
    /// Rows left out because required fields were empty.
    pub skipped: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Insertable)]
#[diesel(table_name = nt_source)]
pub struct NewNtSource {
    pub version: String,
    pub source_file: String,
    pub imported_at: String,
    pub records: i32,
    pub skipped: i32,
}
