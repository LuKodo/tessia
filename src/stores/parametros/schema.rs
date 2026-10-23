diesel::table! {
    parametros (id) {
        id -> Int2,
        max_invoice_amount -> Numeric,
        automatic_subplans -> Jsonb,
        wave_size -> Int2,
        derechos_unavailable_percent -> Int2,
        derechos_min_sample -> Int2,
        cuv_token_failure_percent -> Int2,
        cuv_min_sample -> Int2,
        updated_at -> Timestamptz,
    }
}
