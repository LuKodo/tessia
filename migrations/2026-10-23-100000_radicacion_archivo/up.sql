-- Radicación "archived" marker of a cargue artifact. Go moved a confirmed file
-- into Radicado/; the bucket cannot rename, so the file stays where it is and
-- this row says it was filed. The marker holds only while the artifact still
-- has the same SHA-256 (a regenerated file is a new, unfiled file).
CREATE TABLE radicacion_archivo (
    cargue_id   TEXT        NOT NULL,
    rel_path    TEXT        NOT NULL,
    sha256      TEXT        NOT NULL,
    archived_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (cargue_id, rel_path)
);
