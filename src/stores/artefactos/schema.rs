diesel::table! {
    artefactos (cargue_id, rel_path) {
        cargue_id -> Text,
        rel_path -> Text,
        key -> Text,
        size -> Int8,
        sha256 -> Text,
        content_type -> Text,
        created_at -> Timestamptz,
    }
}
