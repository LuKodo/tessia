use diesel::prelude::*;

use crate::db::schema::nt_records;

/// A persisted Nota Técnica row.
///
/// Read side: used by repository queries and tests, and by the matching phases
/// that come after the load.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Queryable, Selectable)]
#[diesel(table_name = nt_records, check_for_backend(diesel::sqlite::Sqlite))]
pub struct NtRecord {
    pub id: i32,
    /// 1-based row number in the original Excel sheet.
    pub row_number: i32,
    pub cum: String,
    pub cod_generico: String,
    pub forma_farmaceutica: String,
    pub concentracion: String,
    pub unidad_medida: String,
    pub clasificacion: String,
    /// Kept as the original text: the price format is not defined yet.
    pub precio: String,
}

/// A Nota Técnica row that has been validated and is ready to be persisted.
#[derive(Debug, Clone, PartialEq, Eq, Insertable)]
#[diesel(table_name = nt_records)]
pub struct NewNtRecord {
    /// 1-based row number in the original Excel sheet.
    pub row_number: i32,
    pub cum: String,
    pub cod_generico: String,
    pub forma_farmaceutica: String,
    pub concentracion: String,
    pub unidad_medida: String,
    pub clasificacion: String,
    pub precio: String,
}
