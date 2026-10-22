diesel::table! {
    proceso_estado (num_factura, fase) {
        num_factura -> Text,
        fase -> Text,
        cargue_id -> Nullable<Text>,
        estado -> Text,
        motivo -> Text,
        intentos -> Int4,
        version -> Int8,
        claimed_by -> Nullable<Text>,
        claimed_at -> Nullable<Timestamptz>,
        next_attempt_at -> Nullable<Timestamptz>,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}
