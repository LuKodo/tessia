CREATE TABLE nt_records (
    id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    row_number INTEGER NOT NULL,
    cum TEXT NOT NULL,
    cod_generico TEXT NOT NULL,
    forma_farmaceutica TEXT NOT NULL,
    concentracion TEXT NOT NULL,
    unidad_medida TEXT NOT NULL,
    clasificacion TEXT NOT NULL,
    precio TEXT NOT NULL
);
CREATE INDEX idx_nt_records_cum ON nt_records(cum);
