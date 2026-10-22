diesel::table! {
    radicacion_estado (invoice) {
        invoice -> Text,
        folder -> Text,
        step -> Int2,
        status -> Text,
        upload_id -> Text,
        sent_at -> Nullable<Timestamptz>,
        sends -> Int4,
        polls -> Int4,
        step1_reloaded -> Bool,
        reload_for_step2 -> Bool,
        next_attempt_at -> Nullable<Timestamptz>,
        last_detail -> Nullable<Text>,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}
