diesel::table! {
    factura_valor_xml (num_factura) {
        num_factura -> Text,
        valor_total_xml -> Nullable<Numeric>,
        fuente -> Nullable<Text>,
        sincronizado_at -> Nullable<Timestamptz>,
        ruta_zip -> Nullable<Text>,
        xml_valor -> Nullable<Numeric>,
        xml_mtime -> Nullable<Int8>,
        xml_size -> Nullable<Int8>,
    }
}
