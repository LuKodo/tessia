// @generated automatically by Diesel CLI.
//
// NOTE: `diesel_cli` was not available when this file was written, so it was
// authored by hand in the exact form `diesel print-schema` produces for the
// migrations in `migrations/nt_store` and `migrations/load` (both sets create
// the same tables). Regenerate it with `diesel print-schema` once the CLI is
// installed.

diesel::table! {
    nt_records (id) {
        id -> Integer,
        row_number -> Integer,
        cum -> Text,
        cod_generico -> Text,
        forma_farmaceutica -> Text,
        concentracion -> Text,
        unidad_medida -> Text,
        clasificacion -> Text,
        precio -> Text,
    }
}

diesel::table! {
    nt_source (version) {
        version -> Text,
        source_file -> Text,
        imported_at -> Text,
        records -> Integer,
        skipped -> Integer,
    }
}

diesel::allow_tables_to_appear_in_same_query!(nt_records, nt_source,);
