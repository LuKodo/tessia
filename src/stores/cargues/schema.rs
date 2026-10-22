diesel::table! {
    cargues (seq) {
        seq -> Int8,
        id -> Text,
        sha256 -> Text,
        name -> Text,
        origin -> Text,
        date_from -> Nullable<Date>,
        date_to -> Nullable<Date>,
        created_at -> Timestamptz,
    }
}
