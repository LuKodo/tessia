-- Global preparation parameters (a single row, id = 1).
CREATE TABLE parametros (
    id                           SMALLINT      NOT NULL PRIMARY KEY CHECK (id = 1),
    max_invoice_amount           NUMERIC(14,2) NOT NULL CHECK (max_invoice_amount > 0),
    automatic_subplans           JSONB         NOT NULL,
    wave_size                    SMALLINT      NOT NULL DEFAULT 20 CHECK (wave_size BETWEEN 0 AND 500),
    derechos_unavailable_percent SMALLINT      NOT NULL DEFAULT 50 CHECK (derechos_unavailable_percent BETWEEN 0 AND 100),
    derechos_min_sample          SMALLINT      NOT NULL DEFAULT 4  CHECK (derechos_min_sample BETWEEN 0 AND 1000),
    cuv_token_failure_percent    SMALLINT      NOT NULL DEFAULT 50 CHECK (cuv_token_failure_percent BETWEEN 0 AND 100),
    cuv_min_sample               SMALLINT      NOT NULL DEFAULT 4  CHECK (cuv_min_sample BETWEEN 0 AND 1000),
    updated_at                   TIMESTAMPTZ   NOT NULL DEFAULT now()
);
