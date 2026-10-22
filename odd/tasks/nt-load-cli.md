# Feature: nt-load-cli

## Objective
Rust CLI (`facturacion load --nt <file.xlsx>`) that creates an independent "cargue" (own directory + own SQLite `load.db`, Diesel + embedded migrations), imports a Nota Técnica from `.xlsx` into `nt_records`, and prints a summary. Phase 1 only.

## Scope
In: load creation, migrations, Excel NT importer (configurable column mapping), NtRepository (batched, transactional insert), CLI `load --nt`, tracing, tests.
Out: REST API, CSV download/import, cross-matching, invoice lines.

## Constraints
- Stack: clap, calamine, diesel (sqlite) + diesel_migrations, anyhow, thiserror, chrono, uuid, tracing(+subscriber). No Axum, no premature traits.
- Importer knows nothing about Diesel/SQLite. `precio` stays `String`.
- Excel sheet name / column positions / header names are UNKNOWN: centralized, documented, configurable provisional mapping.
- Binary name `facturacion` (package stays `tessia`).
- Existing prototype (`src/nt/*`, old `main.rs`, rusqlite) is replaced. Project is not a git repo: no work-unit commits (recorded as skipped).

## Route
Delegated direct: one writer (trigger: 2+ non-trivial files / preparation). Parent spot-checks.

## Tasks
- [x] T1 Scaffold: Cargo.toml deps + `[[bin]] facturacion`, remove prototype, `migrations/` initial migration, module skeleton
- [x] T2 `db` (connection + embedded migrations + schema) and `load` (Load model + creation service)
- [x] T3 `models/nt_record.rs` (NtRecord/NewNtRecord) + `repositories/nt_repository.rs` (`insert_many`, batches, transaction)
- [x] T4 `importers/nt_importer.rs` + `config` (NtColumnMapping, sheet, header row), domain errors
- [x] T5 `services/nt_service.rs`, `cli.rs`, `main.rs` thin, summary output, tracing init
- [x] T6 Tests; `cargo fmt`, `cargo check`, `cargo clippy` clean; manual e2e with a generated xlsx
- [x] T7 (user request, accepted scope change) Per-step timing: validate, create load (dir+db+migrations), copy file, read Excel, insert into SQLite, total; shown in the final summary ("Tiempos:") and logged with tracing::debug. Route: delegated (same writer, 3 files: load/service.rs, services/nt_service.rs, small timing helper).

- [x] T8 (user decision after inspecting the real file) Real layout: sheet `Hoja1`, header row 1, mapping MED-CUM/COD GEN/FORMA FARMACEUTICA RIPS/CONCENTRACION MEDICAMENTO RIPS/UNIDAD MEDIDA RIPS/TIPO PRODUCTO(clasificacion, inferred)/PRECIO CONTRATO. Rule change: a row with ANY empty required field is NOT saved and does NOT abort the import; the user is warned (summary section + tracing::warn) with row numbers and the empty field(s). Fully empty rows stay silently ignored. Route: delegated (same writer; config, importer, nt_service, load/service + tests).

## Design change (user request, supersedes `load --nt`)
The Nota Técnica is NOT per-cargue. It is global, versioned, and imported separately; every NEW cargue uses the newest one.
- `facturacion nt import <file.xlsx>`: validates + reads the Excel first, builds a NEW global NT database, then archives the previous one and swaps. Layout under the data root: `nt/nt.db` + `nt/nota_tecnica.xlsx` (current), `nt/history/nt_<version>.db` + `nt/history/nota_tecnica_<version>.xlsx` (previous versions, renamed, kept as history). A failed import must never touch the current NT.
- `facturacion load` (no `--nt`): creates the cargue (own dir + own `load.db`) and takes a SNAPSHOT of the current NT into the load's own `nt_records` (+ a one-row `nt_source` with version/source file/records). Rationale: the cargue stays self-contained and reproducible, and the later CSV cross-match happens inside one SQLite as originally planned; an NT replaced later does not alter old cargues. If no NT has been imported: clear error telling to run `nt import` first.
- `FACTURACION_DATA_DIR` now means the data ROOT (default `data`): loads in `<root>/loads`, NT in `<root>/nt`.
- Open interpretation (stated to user, easy to change): versions are kept as separate DB files + xlsx renamed with the version, not as rows in one DB.

- [x] T9 config: data root + `nt_dir`/`loads_dir`; migrations split/organisation for NT store DB (`nt_records` + `nt_source`) and load DB (`nt_records` snapshot + `nt_source`); schema.rs update
- [x] T10 `nt_store` module (mirror of `load/`): `nt import` flow — read first, build new DB in temp name, write `nt_source`, archive previous (db + xlsx) to `history/` renamed by version, swap in; failure leaves current intact; unique version id `YYYYMMDD_HHMMSS_<8hex>`
- [x] T11 `load` rewritten: no `--nt`; requires current NT; snapshot via ATTACH + INSERT..SELECT in the repository layer; summary shows NT version/source/records; timings per step
- [x] T12 CLI (`nt import`, `load`), tests (archive/swap/failure-keeps-current/snapshot isolation/no-NT error), e2e with the real file twice, fmt/check/clippy

## Acceptance
(Superseded by the design change.) `facturacion nt import <file.xlsx>` builds `data/nt/{nt.db,nota_tecnica.xlsx}` and archives the previous version in `data/nt/history/`; `facturacion load` creates `data/loads/<id>/load.db` with a snapshot of the current NT; clear errors for missing file, non-.xlsx, missing column, no NT imported yet, no valid records.

## Progress / Evidence
- T1: `cargo add` resolved diesel 2.3.14 (sqlite), diesel_migrations 2.3.2, libsqlite3-sys 0.38.2 (bundled; `cargo tree -i libsqlite3-sys` shows a single copy shared with diesel), thiserror 2.0.21, chrono 0.4.45, uuid 1.28 (v4), tracing 0.1.44, tracing-subscriber 0.3.23 (env-filter); dev-dep tempfile 3. `[[bin]] facturacion`. `src/nt/` deleted. Migration dir `migrations/2026-10-20-120000_create_nt_records/` (diesel timestamp format). `cargo check`: ok.
- T2: `db::connection` (establish / run_migrations via `embed_migrations!`), hand-written `db/schema.rs` (diesel_cli not installed; noted in file), `load::{model,service}`. Load id uses local time (chrono `Local`). `cargo check`: ok.
- T3: `models::nt_record`, `repositories::nt_repository` (batch 100, single transaction). Tests incl. 250 rows across batches, rollback via SQLite trigger, index exists. `cargo check`: ok.
- T4: `config` (loads_dir with `FACTURACION_DATA_DIR`, `NtImportConfig::default()` PROVISIONAL), `importers::nt_importer`. RED observed: with `cell_to_string` as `todo!()`, `cargo test importers` -> 4 passed, 15 failed; after implementing -> 19 passed.
- T5: `services::nt_service::import`, `cli`, thin `main`, tracing (stderr, `RUST_LOG`, default warn).
- T6: `cargo fmt --check`: ok; `cargo check`: ok; `cargo clippy --all-targets -- -D warnings`: clean; `cargo test`: 33 passed, 0 failed.
- E2E (python zipfile-generated xlsx in mktemp, `FACTURACION_DATA_DIR` in another mktemp): ok file (250 rows + 1 empty row, numeric CUM, mixed-case/padded headers) -> "Registros leídos: 250 / insertados: 250"; sqlite3: count 250, index `idx_nt_records_cum` present, empty Excel row 12 skipped (row_number jumps 11 -> 13). Error paths (exit 1): missing file, `.csv`, missing CUM column ("No se encontró la columna CUM"), empty CUM ("Fila 6: campo CUM vacío"). Repo `data/` not created.
- T7: new `src/timing.rs` (`StepTimings` record/time, `measure`, `format_duration`), timings measured in `load/service.rs` and `services/nt_service.rs` (`NtImportSummary` gained `read_elapsed`/`insert_elapsed`), "Tiempos:" section printed after the summary only on success; per-step `tracing::debug!(elapsed_ms)`; `tracing::error!` on failed load includes `elapsed_ms`. `cargo fmt --check`, `cargo check`, `cargo clippy --all-targets -- -D warnings`: clean; `cargo test`: 41 passed. E2E (5000-row generated xlsx): Tiempos block printed (Validación < 1 ms, Creación del cargue 2 ms, Copia < 1 ms, Lectura 202 ms, Inserción 63 ms, Total 270 ms); missing-column error path: no timings on stdout, exit 1, message unchanged.
- T8: `NtImportConfig::default()` = real layout (sheet `Hoja1`, header row 1, MED-CUM / COD GEN / FORMA FARMACEUTICA RIPS / CONCENTRACION MEDICAMENTO RIPS / UNIDAD MEDIDA RIPS / TIPO PRODUCTO (inferred, unconfirmed) / PRECIO CONTRATO). Importer returns `NtImportResult { records, skipped: Vec<SkippedRow> }` (all empty labels per row; `Fila N: campo X vacío` / `campos X, Y vacíos`); the hard `EmptyField` error was removed; fully empty rows ignored. `NtImportSummary.read = inserted + skipped.len()`, per-row `tracing::warn!`, summary prints `Registros omitidos: N` always plus an "Advertencias:" section (cap 20, then `... y N más`) on stdout; "Tiempos:" stays last. Tests: compile-RED observed (tests written before the types existed), then 51 passed; `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` clean. E2E fixture (hidden sheet NT first + Hoja1, 300 rows, 4 problem rows): read 300 / inserted 296 / omitted 4, skipped rows absent from `nt_records`, exit 0; missing column still exit 1. REAL file (read-only, source md5 unchanged): read 6783, inserted 6768, omitted 15 (rows 1735, 1879, 4995, 6740, 6742, 6743, 6745, 6746, 6747, 6749, 6756, 6757, 6758, 6762, 6766), exit 0, 492 ms total.
- T9: `config::DataLayout` (root from `FACTURACION_DATA_DIR`, default `data`; `nt/`, `nt/history/`, `loads/`). Migrations: two embedded sets `migrations/nt_store/` and `migrations/load/` (each: existing `create_nt_records` moved/copied unchanged + `create_nt_source`) because the load DB will grow its own tables later while the NT store will not; `db::connection::{run_nt_store_migrations, run_load_migrations}`. `nt_source.version` is `TEXT PRIMARY KEY NOT NULL` (Diesel needs a primary key); `schema.rs` hand-updated; `diesel.toml` points at `migrations/load`. Shared id generator `timing::new_timestamped_id()` (local time) used for load and NT version ids.
- T10: `nt_store::{model,service}`: validate -> read Excel (`nt_service::read`) -> zero valid records = error `La Nota Técnica no contiene registros válidos` -> build `.nt_new.db`/`.nota_tecnica_new.xlsx` in `nt/` (leftovers cleared first), `nt_service::store` + `nt_source` row, connection dropped, then `swap_in`: archive current (version read from its `nt_source`; unreadable/unsafe -> fresh `new_timestamped_id()`) to `history/`, move staged files in; `rename_all` undoes completed renames if one fails; existing history file = error before moving anything. Extra timed step "Copia del archivo" added. Per-row skip logs are now `tracing::debug!`.
- T11: `load/service.rs` rewritten: `load` has no args; checks `nt/nt.db` before creating anything; `NtRepository::snapshot_from` (ATTACH with bound path, transaction around INSERT..SELECT of records ordered by id + `nt_source` copy with count verification, DETACH always attempted); the xlsx is not copied into the load; summary shows version / source file / records.
- T12: CLI `nt import ARCHIVO` + `load`; tests added (id format/uniqueness, first import, archive-by-old-version, failing imports leave current byte-identical and create no history, leftovers cleanup, unreadable previous DB fallback, rename rollback, history collision, no-NT error without load dir, snapshot version/count, snapshot isolation across imports, ATTACH with space/quote path, DETACH after failure, migrations create tables+index in both sets). Test xlsx fixtures are written by a small `zip`-based helper (`nt_store::test_support`, dev-dependency `zip`). `cargo fmt --check` ok, `cargo clippy --all-targets -- -D warnings` clean, `cargo test` 68 passed, `cargo build` ok. E2E with the REAL file (md5 unchanged): `load` on empty root -> message + exit 1 + no `loads/`; `nt import` read 6783 / inserted 6768 / omitted 15; `load` -> 6768 records, nt_source row, `idx_nt_records_cum`, only `load.db` in the load dir; second `nt import` -> `history/` has `nt_<v1>.db` + `nota_tecnica_<v1>.xlsx`, current is v2; second `load` uses v2 while the first load keeps v1; missing file / `.csv` / missing column -> exit 1 with current files and history unchanged.
- No git repo: work-unit commits skipped; no native review.

## Next step
Parent spot-check; adjust provisional headers in `src/config/mod.rs` once the real Excel layout is known.
