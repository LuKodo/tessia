CREATE TABLE nt_source (
    version TEXT PRIMARY KEY NOT NULL,
    source_file TEXT NOT NULL,
    imported_at TEXT NOT NULL,
    records INTEGER NOT NULL,
    skipped INTEGER NOT NULL
);
