diesel::table! {
    dataset_preparacion (cargue_seq) {
        cargue_seq -> Int8,
        load_id -> Text,
        load_name -> Text,
        load_from -> Nullable<Date>,
        load_to -> Nullable<Date>,
        load_origin -> Text,
        source -> Text,
        max_amount -> Nullable<Numeric>,
        automatic_subplans -> Jsonb,
        nt_version -> Text,
        prepared_format -> Bool,
        stats -> Jsonb,
        skipped_lines -> Jsonb,
        prepared_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    dataset_factura (cargue_seq, factura) {
        cargue_seq -> Int8,
        factura -> Text,
        factura_norm -> Text,
        estado -> Text,
        codigo -> Text,
        motivo -> Text,
        id_mipres -> Text,
        orden -> Int4,
        conciliacion -> Nullable<Jsonb>,
        version -> Int8,
        updated_at -> Timestamptz,
        fecha_factura_txt -> Text,
        ssc -> Text,
        subplan -> Text,
        autorizacion -> Text,
        valor_convenio -> Nullable<Numeric>,
    }
}

diesel::table! {
    dataset_linea (cargue_seq, factura, orden) {
        cargue_seq -> Int8,
        factura -> Text,
        orden -> Int4,
        source_line -> Int4,
        total_value -> Text,
        data -> Jsonb,
    }
}

diesel::table! {
    dataset_actual (load_id) {
        load_id -> Text,
        cargue_seq -> Int8,
        updated_at -> Timestamptz,
    }
}
