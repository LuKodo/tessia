-- Cache of validated affiliate rights (complete records only).
CREATE TABLE derechos_afiliado (
    tipo_documento   TEXT        NOT NULL,
    numero_documento TEXT        NOT NULL,
    tipo_usuario     TEXT        NOT NULL,
    zona             TEXT        NOT NULL,
    municipio_dane   TEXT        NOT NULL,
    sexo             TEXT        NOT NULL,
    fecha_nacimiento TEXT        NOT NULL,
    validado_at      TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (tipo_documento, numero_documento)
);
