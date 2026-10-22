diesel::table! {
    derechos_afiliado (tipo_documento, numero_documento) {
        tipo_documento -> Text,
        numero_documento -> Text,
        tipo_usuario -> Text,
        zona -> Text,
        municipio_dane -> Text,
        sexo -> Text,
        fecha_nacimiento -> Text,
        validado_at -> Timestamptz,
    }
}
