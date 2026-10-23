# Architecture (foundation, task T1)

Rust port of a subset of the Go `tessia` CLI (`/Users/adatospharma/Desktop/TessiaAutomatic/tessia`,
read-only reference) onto **Postgres + Diesel**, an **object store** (the Cassandra storage API) and a
**read-only Redis Nota Técnica (NT)**, so the same cargue can be processed by N machines at once.
Binary: `facturacion` (package/library `tessia`). Synchronous Rust: OS threads + blocking `reqwest`.

Binding specs are in `odd/tasks/migracion-go-postgres.md` (S1-S9). This file is the map for the phase
writers that start from the foundation.

## Contents

1. [Build prerequisites](#build-prerequisites)
2. [Module map](#module-map)
3. [Public API by module](#public-api-by-module)
4. [Dataset store](#dataset-store)
5. [How a phase is written](#how-a-phase-is-written)
6. [Adding tables and migrations](#adding-tables-and-migrations)
7. [Writing tests](#writing-tests)
8. [Conventions](#conventions)
9. [Go behavior intentionally not ported](#go-behavior-intentionally-not-ported)

## Build prerequisites

* A stable Rust toolchain (edition 2024) and a C compiler (`clang`/`cc` + `make`).
* **No system libpq is needed**: `pq-sys` is built with the `bundled` feature (verified on macOS
  arm64; SCRAM-SHA-256 authentication works). If a platform ever fails to build it, drop the
  `pq-sys` line from `Cargo.toml` and point the linker at a system libpq instead, e.g.
  `PQ_LIB_DIR=/opt/homebrew/opt/libpq/lib cargo build` (Homebrew `libpq`).
* Tests need two local services (no other container or database is touched):

  | service | default URL | override |
  |---|---|---|
  | TEST Postgres | `postgres://postgres:postgres@127.0.0.1:55432/postgres` (admin URL) | `TESSIA_TEST_PG_ADMIN_URL` |
  | TEST Redis | `redis://127.0.0.1:56379` | `TESSIA_TEST_REDIS_URL` |

  Every test gets its own throwaway database (`TestDb`) and its own random Redis key prefix, both
  removed afterwards. The server allows ~100 connections shared by all test binaries; keep test pools
  small (`TestDb` uses 6).
* Running: set `TESSIA_DATABASE_URL`, `TESSIA_REDIS_URL`, `TESSIA_CASSANDRA_BASE_URL`,
  `TESSIA_CASSANDRA_API_KEY`, `TESSIA_CASSANDRA_BUCKET` (or `--config file.env`), then
  `facturacion db migrate` (also done automatically when any phase command starts).

## Module map

```
src/
  main.rs            thin: tracing to stderr (RUST_LOG), parse, dispatch, "Error: {err:#}" -> exit 1, usage -> exit 2
  lib.rs             exposes every module (integration tests import `tessia::...`)
  cli.rs             top-level clap tree; each phase owns its Args in phases/<name>/cli.rs
  build.rs           re-embeds migrations when migrations/ changes
  config/            Settings (get/require/is_set), key registry, env file, `config check`
  db/                DbPool, DbError (+ is_transient), with_retry, embedded migrations, TestDb, `db migrate`
  domain/            pure types: invoice normalization, Phase/PhaseState/Outcome, cargue id, radicacion states,
                     dataset (Dataset/Invoice/Line + test builders)
  stores/            Diesel stores, one directory (and schema.rs) each:
                     cargues, artefactos, proceso, radicacion, derechos, parametros, valor_xml, leases, dataset
  storage/           ObjectStore trait, CassandraStore, MemoryStore, Artifacts service, Workdir
  nt/                NtCatalog trait, RedisNt, MemoryNt, NtRow
  http/              RetryPolicy + send_with_retry, AdaptiveLimiter (+ per-system numbers), TokenCache, client builder
  util/              gson (Gson-compatible JSON), fsx, decimal, text, par (thread pool map)
  context.rs         AppContext (everything a phase gets)
  shutdown.rs        cooperative Ctrl-C flag
  testing/           MockServer (tiny_http) for HTTP tests         [cfg(test) or feature "test-support"]
  timing.rs          StepTimings / format_duration ("Tiempos:" section)
  phases/            one directory per command (skeletons): common.rs + the 11 phase modules
  fevxml/            placeholder; the shared XML library (T2) overwrites it
migrations/          Diesel migrations (foundation: 2026-10-22-0001xx..0008xx)
tests/               integration tests (cli.rs runs the real binary, foundation.rs uses the lib)
```

Data flow of a phase:

```
 proceso_estado (Postgres)  ──claim──►  worker threads  ──►  Workdir (local temp, deleted on drop)
        ▲  save_outcome (CAS)                │ HTTP (retry + AIMD limiter)        │ publish / fetch
        │                                    ▼                                    ▼
   NtCatalog (Redis, read-only)         external systems              Artifacts ─► bucket + `artefactos` index
```

## Public API by module

### `config`

```rust
use tessia::config::Settings;

let settings = Settings::load(cli.config.as_deref())?;      // defaults < file < TESSIA_* env; empty never overrides
settings.get("redis.nt_prefix");                            // Option<&str>, default "nt:"
settings.get_or("redis.nt_prefix", "nt:");
settings.is_set("cassandra.api_key");
settings.require(&["database.url", "redis.url"])?;           // Err names keys + env vars, never values
let url = settings.require_one("database.url")?;
settings.list("zonaser.roles");                              // comma separated -> Vec<String>
settings.workdir_root();                                     // workdir.root or <os temp>/tessia-work
let s = Settings::from_pairs([("TESSIA_REDIS_URL", "redis://x")]); // tests
```

* Env var of key `a.b_c` is `TESSIA_A_B_C`. The default file (`--config` / `TESSIA_CONFIG`) uses the
  same names; unknown names are an error, except the retired Go names in `RETIRED_ENV_NAMES`.
* **To add a key, append it to `KEYS` in `src/config/keys.rs`** (in your phase's section; `secret: true`
  for credentials/personal data). New infrastructure keys: `database.url`, `database.pool_size`,
  `redis.url`, `redis.nt_prefix`, `cassandra.base_url|api_key|bucket`, `workdir.root`.
* `facturacion config check [key...]` prints the table of keys (set/unset, secret) and fails when a
  requested key is missing.

### `db`

```rust
let pool = DbPool::from_settings(&settings)?;               // r2d2, verifies one connection
pool.run("op name", |conn| { /* one idempotent statement */ Ok(()) })?;      // retried on transient errors
pool.transaction("op name", |conn| { /* unit of work */ Ok(()) })?;          // whole unit re-run on deadlock/40001/connection loss
```

* `DbError::{Query, Pool, Connect, Other}` with `is_transient()` (40001, deadlock, closed/lost
  connection, pool timeout) and `is_unique_violation()`. `?` converts Diesel errors. `with_retry`
  (4 runs, jittered 50 ms doubling backoff) is what `run`/`transaction` use; closures must be safe to
  repeat and must not have other side effects.
* `migrate::run_pending(conn)` / `migrate::migrate_url(url)` apply embedded migrations under a
  Postgres advisory lock (several machines may start at once); idempotent. CLI: `facturacion db migrate`.
* `test_support::TestDb` (see [Writing tests](#writing-tests)).

### `domain`

```rust
normalize_invoice(" evs-12 ") == "EVS12";                   // trim, drop '-', upper case: key of every invoice table
Phase::Rips.as_str() == "rips";                             // exact Go names: convenio derechos thefactory fev_ajuste fev_valor conciliacion rips cuv soporte
PhaseState::EnProgreso.as_str() == "EN_PROGRESO";           // DONE EXCLUDED REVISION_MANUAL FAILED EN_PROGRESO
Outcome::done() / failed("MOTIVO") / excluded("..") / manual_review("..");   // { state, motivo }
phase::gate(own_state, Some((Phase::FevAjuste, state)));    // Option<String>: "REVISION_MANUAL" | "<PHASE>_NO_OK"
cargue::load_context("Convenio_21102026_21102026.csv");     // LoadInfo { id, name, from, to, origin }
radicacion::{Step, RadStatus, RadState}                     // is_done / is_terminal / is_finished (Go Finished)
```

Cargue ids are the folder-safe CSV stem (S8); they never contain `__` (the bucket key separator).

### `stores`

All take a `DbPool` (`Store::new(pool)`; `AppContext` has accessors). Timestamps come from the database
clock. Money is `rust_decimal::Decimal` / `NUMERIC`.

| store | table | main API |
|---|---|---|
| `CarguesStore` | `cargues` (seq PK, unique `(id, sha256)`) | `ingest(&NewCargue)` -> `Ingested{cargue, created}` or `IngestError::Superseded`; `newest(id)`, `get(id, sha)`, `recent(since, limit)` |
| `ArtefactosStore` | `artefactos` (PK `cargue_id, rel_path`) | `upsert`, `get`, `list(cargue)`, `delete` (use through `Artifacts`) |
| `ProcesoStore` | `proceso_estado` (PK `num_factura, fase`) | `load`, `list_by_cargue`, `start_all`, `claim_available`, `save_outcome`, `release`, `save`/`save_all`, `record`, `insert_outcome`, `save_if_current` |
| `RadicacionStore` | `radicacion_estado` | `load(&[inv])`, `save(&RadState)`, `finished(&[inv])` (chunked, normalized) |
| `DerechosCache` | `derechos_afiliado` | `get(doc_type, doc_number, not_before)`, `put(.., &Rights, at)` |
| `ParametrosStore` | `parametros` (singleton `id = 1`) | `get`, `get_or_default(fallback_amount)`, `set` (validates like Go) |
| `ValorXmlStore` | `factura_valor_xml` | `sync_xml_total`, `stored_xml_total` (gate); `upsert_xml_values`, `stored_xml_values` (tracking) |
| `LeaseStore` | `leases` | `acquire`, `renew`, `release`, `holder`, `guard` (RAII) |
| `DatasetStore` | `dataset_*` | prepared convenio dataset: see [Dataset store](#dataset-store) |

**Claims (`ProcesoStore`) are the multi-machine contract** (S3):

```rust
let store = ctx.proceso();
let loaded = store.load(Phase::Rips, Some(&invoices))?;
let items: Vec<ClaimItem> = invoices.iter().map(|i| ClaimItem::from_loaded(i, loaded.get(i))).collect();
let claimed = store.claim_available(Phase::Rips, &items, &ctx.claim_options(&cargue_id))?; // invoice -> version
for (invoice, version) in claimed {
    // ... work ...
    if !store.save_outcome(Phase::Rips, &invoice, &Outcome::done(), version)? {
        // our claim was taken over (stale TTL): drop the result
    }
}
```

* `claim_available`: partial claim, what other machines have not taken (split the work).
  `start_all`: Go `StartAll`, all-or-nothing (`None` when any item is lost, nothing retained).
* The claim is a compare-and-set on the state/version you loaded; no row = insert (`version 0`).
* An `EN_PROGRESO` row older than `ClaimOptions::stale_after` (`DEFAULT_CLAIM_TTL` = 30 min) can be taken
  over by passing it as the expectation; the old owner's late `save_outcome` returns `false`.
* `save_outcome` writes only while the row is `EN_PROGRESO` at the claimed version, bumps `intentos`
  (Go `Record`: attempts continue) and clears the claim. `release` is Go `CancelStarted`.
* `leases` replace the `.tessia-*.lock` files (e.g. one `diario:<date>` lease for the cron entry point);
  call `renew` while working longer than the TTL.

### `storage`

```rust
let artifacts = ctx.artifacts();                             // Artifacts over ctx.objects + `artefactos`
artifacts.publish_bytes("Convenio_21102026_21102026", "derechos/FEV1.json", &json)?;
artifacts.publish_file(cargue, "CRC/CRC_806_FEV1.zip", &path)?;
let bytes = artifacts.fetch_bytes(cargue, "derechos/FEV1.json")?;        // SHA-256 + size verified
artifacts.fetch_to_file(cargue, "XML/FEV1_XML_PDF.zip", &dest)?;
let wd = ctx.workdir(cargue)?;                               // <workdir.root>/<cargue>-<rand>/, 0700, deleted on Drop
artifacts.fetch_into(&wd, Some(layout::XML))?;               // recreate the folder layout from the bucket
wd.write("CPSCAN/FEV1.cpscan.json", b"{}")?;                 // 0600, atomic, no ".."
artifacts.publish_workdir(&wd, None)?;                       // everything under the workdir
```

* Bucket key = `<cargue_id>__<rel_path with "/" -> "__">` (`object_key`). Rel paths may not contain `__`.
  Never parse keys: `artefactos` is the index.
* `trait ObjectStore { put_bytes, put_file, get_bytes, get_to_file, exists, delete, list(start, limit), list_prefix }`
  implemented by `CassandraStore` (contract in its module docs: upload `POST .../files/upload`
  multipart, `exists` = download with the body dropped, Bearer token never logged, uploads retried
  because same-name upload overwrites) and `MemoryStore` (tests).
* `layout::{XML, CRC, CPSCAN, DERECHOS, RADICADO}` are the Go folder names.

### `nt`

```rust
let nt = &ctx.nt;                                            // Arc<dyn NtCatalog>
let version = nt.current_version()?.expect("NT version");
let rows: Vec<NtRow> = nt.lookup(&version, "cum_facturar", "19900001-1")?;   // via the idx sets, `<id>#N` variants included
let one = nt.row(&version, "1234#2")?;
nt.meta(&version)?;
```

`NtRow` has typed optional fields (`cum_facturar cod_auf cod_mutual cod_gen_siesa tipo_producto ffm_rips
umm_rips upr_rips precio_contrato precio_final precio_maximo subplan contrato clasificacion estado`,
prices as `Decimal`) plus `extra` for any other key, `id` (hash field) and `get(field)`. Indexed fields:
`cod_auf cod_mutual cum_facturar contrato estado`. `RedisNt` is read-only; `MemoryNt::new().with_version(..)`
for tests.

### `http`

```rust
let client = http::build_client(&ClientOptions::default().with_cookies().with_user_agent(http::MEDICAR_USER_AGENT))?;
let limiter = ctx.limiters.get("thefactory");               // Arc<AdaptiveLimiter>, shared process-wide
let resp = http::send_with_retry(&RetryPolicy::default(), limiter.as_deref(), || client.post(url).json(&body))?;
```

* `send_with_retry(policy, limiter, build)`: `build` runs once per attempt (body re-sendable). Retries
  transport errors, 408/425/429/5xx with exponential backoff (250 ms doubling to 10 s, +-20% jitter,
  `Retry-After` honored and capped). POST/PATCH are retried only on 425/429 unless
  `retry_non_idempotent` or an `Idempotency-Key` header. Exhausted status retries return the last
  response as `Ok`; only transport errors are `Err`. Messages never contain the query string.
  `RetryPolicy::mutualser_upload()` = 10 attempts, 1 s to 2 min.
* `AdaptiveLimiter` (AIMD): `acquire()` -> `Permit` (`finish(healthy)`; a dropped permit counts as a
  failure); `exchange(|| Ok((status, value)))` holds a slot while the body is read. Numbers per system
  (`limiter::specs`): TheFactory 4/20/30 window 100 pause 40..900 ms; SISPRO 8/20/40 w80 25..900;
  CPScan 6/24/35 w100 25..700; Mutual Ser 8/20/40 w80 25..900; derechos 4/16/`max` w80 25..900.
* `TokenCache` (single-flight cached bearer tokens), `send_with_auth` (one re-login on 401),
  `jwt_expiry`. Timeouts: 60 s request, 10 s connect (SISPRO used 210 s: pass
  `ClientOptions::with_timeout`). `*.ca_file` settings -> `ClientOptions::with_ca_file`.
* Not ported yet: SHA-256 leaf pinning (`minsalud.tls_pin_sha256`) needs a custom rustls verifier;
  the SISPRO writer adds it if still needed.

### `util`

* `gson`: `Json` tree + `to_string(&Json, &GsonOptions)` / `to_string_of(&serde_value, ..)`; 2-space pretty
  output, member order kept, nulls dropped unless `serialize_nulls`, no HTML escaping by default,
  integers plain, floats like `Double.toString`, `Json::Number` verbatim, `Json::decimal(Decimal)`.
* `fsx`: `atomic_write` (temp + rename, 0600), `write_private`, `ensure_dir` (0700), `move_file`
  (cross-device fallback), `safe_join`.
* `decimal`: `parse_max_amount`, `parse_agreement_amount` (`1.234,56` or `1234.56`), `format2`, `round2`, `sum`.
* `text::normalize_key`: upper case + strip accents + collapse spaces (Go `catalogHeader`).
* `par::map(items, workers, f)`: bounded thread map, results in input order.

### `context`

```rust
let ctx = AppContext::from_settings(settings)?;   // connects, migrates, builds CassandraStore + RedisNt (lazy)
ctx.settings; ctx.pool; ctx.objects; ctx.nt; ctx.workdir_root; ctx.owner /* host:pid:uuid */; ctx.limiters;
ctx.proceso(); ctx.cargues(); ctx.radicacion(); ctx.derechos_cache(); ctx.parametros(); ctx.valor_xml(); ctx.leases(); ctx.dataset();
ctx.artifacts(); ctx.workdir(&cargue_id)?; ctx.claim_options(&cargue_id);
// tests: AppContext::for_tests(&db).with_nt(Arc::new(MemoryNt::..)).with_objects(Arc::new(MemoryStore::new()))
```

`facturacion config check` and `db migrate` run without building the context (no Redis/bucket needed).

### `phases`

`phases/common.rs` is the contract with `proceso run` (T13):

```rust
pub struct PhaseRequest { pub cargue_id: String, pub invoices: Option<Vec<String>>, pub workers: Option<usize> }
pub struct PhaseReport { pub outcomes: Vec<InvoiceOutcome>, pub skipped: Vec<(String, String)> } // .counters(), .summary()
pub fn run_phase(ctx: &AppContext, req: &PhaseRequest) -> anyhow::Result<PhaseReport>;   // in each phase module
pub fn run(ctx: &AppContext, args: Args) -> anyhow::Result<()>;                           // command line entry
```

`fev` exposes `run_ajustar_phase` and `run_verificar_phase`; `convenio_descargar` and `proceso` have no
`run_phase`. `CommonArgs` (`--cargue`, `--invoices a,b`, `--workers N`) can be flattened into a phase's
clap `Args`. Each module's `cli.rs` already defines `Args` (struct, or struct + `Command` enum for
`fev ajustar|verificar`, `cuv generar|barrer`, `radicacion run|recheck-step2`, ...) and is wired in `crate::cli`.

## Dataset store

The prepared convenio dataset (Go `cargue_preparacion`, `cargue_factura`, `cargue_factura_linea`,
`cargue_actual`; `ValueTables.AgreementTotal`) in Postgres. `convenio preparar` writes it, `conciliar`
rewrites invoices of it, every other phase reads it. Code: `src/domain/dataset.rs` (pure types, test
builders), `src/stores/dataset/` (store, `schema.rs`, `test_support.rs`), migration
`migrations/2026-10-23-000000_dataset_tables/`. `ctx.dataset()` gives the `DatasetStore`.

### Identity: "cargue" is `cargues.seq`

Every method takes `cargue: i64`, the `seq` of an upload in `CarguesStore` (a `Dataset` carries it in
`ds.cargue`). A phase that only knows the load id (`PhaseRequest.cargue_id`, e.g.
`Convenio_21102026_21102026`) resolves it with `current_cargue(load_id)` or reads straight through
`read_current(load_id, subset)`. *Current* = the cargue the last `replace` made current (a pointer per load,
`dataset_actual`), else the newest upload: an ingested-but-not-yet-prepared upload never hides the previous
prepared dataset (Go `cargue_actual`). `ErrCargueSuperseded` stays in `CarguesStore::ingest`.

### Types (`tessia::domain::dataset`)

```rust
Dataset     { cargue: i64, preparation: Preparation, invoices: Vec<Invoice>, excluded: Vec<Excluded>, manual_review: Vec<ManualReview> }
Preparation { load: LoadInfo, source, max_amount: Option<Decimal>, automatic_subplans: Vec<String>,
              nt_version: String /* Redis NT version pinned at prepare time */, prepared_format: bool,
              prepared_at: Option<DateTime<Utc>> /* None on write = DB now() */, stats: Stats, skipped_lines: Vec<i64> }
Invoice     { invoice /* normalized */, lines: Vec<Line>, conciliacion: Option<InvoiceAudit>, id_mipres, version: i64 }
Line        { source_line, nit, plan, subplan, invoice, invoice_raw, doc_type, doc_number, beneficiary, point_of_sale,
              formula_number, ssc, formula_type, date, lot, plu, article, quantity, unit_value, unit_cost, total_value,
              copay, authorization, mipres, agreement_cum, diagnosis_code, diagnosis_name, causal_exento, ips,
              prescriber_id, prescriber_name, gender, birth_date, department, municipality, classification, cum,
              product_type, pharmaceutical_form, concentration, measure_unit, agreed_price,
              conciliacion: Option<LineAudit>, extra: BTreeMap<String, serde_json::Value> }
Excluded { invoice, reason }   ManualReview { invoice, code, reason } (+ .text() = "<code>: <reason>")
Stats (Go Stats)   InvoiceState::{Prepared "PREPARADA", Excluded "EXCLUIDA", ManualReview "REVISION_MANUAL"}
```

* `Line` has **exactly the Go fields**, all `String` (the verbatim trimmed CSV text, e.g. `"1.234,56"`):
  RIPS/soporte/conciliar print and compare them as written. For numbers use the accessors
  `line.total_value_amount()`, `unit_value_amount()`, `copay_amount()`, `agreed_price_amount()` ->
  `Result<Decimal, AmountError>` (`parse_agreement_amount` rules) and `invoice.agreement_total()`.
  Anything without a typed field goes into `line.extra` (flattened JSON, round-trips).
* `Line.invoice` and `Invoice.invoice` are the normalized number (`normalize_invoice`). `Line.date` etc. keep
  the file's text.
* `Invoice.version` is the optimistic-locking token: set by every read, `0` on invoices built in memory, ignored
  by `replace`. `Dataset::{all_names, invoice(name), filtered(names)}` (`filtered` = Go `FilterDataset`),
  `subset_names`, `sum_line_totals`, `Line::affiliate_key()`, `Invoice::subplan()` are the small helpers.
* `max_amount: None` means "unset": fall back to `ctx.parametros().get_or_default(..)` (Go: dataset value, else
  `convenio.max_invoice_amount`).
* Storage: `dataset_linea.data` is the whole `Line` as JSONB (no 40-column table); `total_value` is copied
  out for the agreement total. `dataset_factura` keeps `estado`, `orden` (one sequence for the three states),
  `version`, `id_mipres`, `conciliacion` (JSONB).
* **Summary columns** (dashboard read models, migrations `2026-10-24-000100_dataset_factura_summary` and
  `..000200_..._backfill`): Go filters, sorts and aggregates the dashboard over stored columns of
  `cargue_factura`, so `dataset_factura` carries the same five: `fecha_factura_txt` (first non-blank line
  `date`, trimmed, 10 characters), `ssc`, `subplan`, `autorizacion` (first non-blank line value, trimmed) and
  `valor_convenio NUMERIC(18,2) NULL` (sum of the line totals with the `parse_agreement_amount` rules; `NULL`
  when there are no lines or one total is unreadable). Chosen over deriving them from `dataset_linea.data` at
  read time because the JSON route reads every line of every invoice per poll and cannot parse `1.234,56` in
  SQL. Trade-off: five more columns per invoice and a rule that must stay in one place:
  `stores::dataset::summary::summarize` is the only Rust implementation, called by `replace` (every write
  path of `replace`, `replace_touching` and merge) and `update_invoices` (lines change, so the columns are
  rewritten with them); the backfill migration applies the same rules in SQL to the rows stored before, and
  `tests/dataset_store.rs` proves both give the same columns. Excluded and manual-review invoices have no
  lines, so their columns stay empty (as in Go). Nothing else writes them: code that changes lines must go
  through those two methods.

### `DatasetStore` methods

```rust
let store = ctx.dataset();            // DatasetStore::new(pool)

// --- writes (preparar / conciliar)
store.replace(&ds, false)?;           // whole dataset of ds.cargue; makes it the current cargue of its load
store.replace(&ds, true)?;            // merge: invoices named in ds replace the stored ones, the rest are kept
store.replace_touching(&ds, &requested)?; // merge with an explicit `touched` list (Go `--invoices`; also drops names ds lacks)
store.update_invoices(cargue, &[(inv, inv.version)])?; // optimistic update, see below
store.save_preparation(cargue, &preparation)?;         // header only (nt_version, stats, ...); does not move the pointer

// --- reads
let ds: Dataset = store.read(cargue, None)?;                       // everything; Err(NotFound) if no dataset
let ds = store.read(cargue, Some(&["EVS1".into()]))?;              // subset (see rules below)
let invoices: Vec<Invoice> = store.read_invoices(cargue, Some(&names))?; // prepared invoices WITH version
let ds: Option<Dataset> = store.read_current("Convenio_21102026_21102026", Some(&names))?;
let header: Option<Preparation> = store.preparation(cargue)?;       // no rows read
let names: Vec<String> = store.invoice_names(cargue)?;             // prepared, stored order
let excluded = store.names_with_state(cargue, InvoiceState::Excluded)?;
let seq: Option<i64> = store.current_cargue("Convenio_21102026_21102026")?;
let total: Option<Decimal> = store.agreement_total("evs-1")?;       // radicacion CheckValue
```

Rules ported from Go:

* **`replace`** is one transaction: header upsert, invoices (position = prepared, then excluded, then manual
  review, each in its order), lines, then the current-cargue pointer (a failed replace leaves the previous
  current cargue). An invoice that was stored before comes back with `version + 1`; new ones start at `0`.
  `ds` must not name an invoice twice (`DatasetError::Invalid`). `merge = true` (or a non-empty `touched`)
  keeps the stored invoices not named, appends the new ones, keeps the **stored** `stats`/`skipped_lines`, ORs
  `prepared_format`, takes the rest of the header from `ds`; a cargue with no dataset or stored for another
  `load.id` is replaced as a whole. `ds.cargue` must exist in `cargues` (`DatasetError::CargueNotFound`).
* **Subset reads** (`subset: Option<&[String]>`): `None` = everything; `Some(names)` = names normalized with
  `normalize_invoice`, distinct, unknown ones absent; `invoices`/`excluded`/`manual_review` hold only those,
  in stored order (not request order), with lines and versions; the header (`preparation`) is always the
  whole dataset's. `Some(&[])` reads the header only. Equal to `read(..)` + `Dataset::filtered(..)` but only
  the requested rows travel. A per-invoice phase (`--invoices <one>`) must use this.
* **`agreement_total(invoice)`**: sum of `total_value` of the invoice's lines in the newest *current* cargue
  that holds it as `PREPARADA` (highest cargue seq wins among loads; a PREPARADA invoice without lines is
  `0`; excluded/manual/unknown is `None`). It is never stored. A line total that is not a readable amount is
  `Err(DatasetError::Invalid(..))` (radicacion maps it to `VALIDACION_VALOR_NO_DISPONIBLE`). Two decimals.
* Snapshots: `read` runs in one read-only repeatable-read transaction, so a concurrent `replace` is seen
  whole or not at all.

### Dashboard read queries (`stores::dataset::dashboard`)

Read-only queries behind `GET /api/cargues`, `/api/resumen`, `/api/seguimiento/consulta` and
`/api/facturas/{f}/seguimiento` (rules in `api::read::derive`, routes in `api::read::{cargues,resumen,consulta,tracking}`):
`dashboard_loads(limit)` (current upload of every load; unprepared ones included), `dashboard_rows()` (every invoice
of every current prepared cargue with its summary columns), `dashboard_prepared_names(seqs)`, `dashboard_native(scope)`
(per invoice: filing row, the four artifact phase states, latest phase update; `last_detail` is only transferred
when the rules can read it: status `BLOCKED` or the text holds `22`), `dashboard_extras(names)` (texts, `xml_valor`,
`portal_cargue` of one page) and `dashboard_invoice(norm, load)`.

Design choice and trade-off: Go classifies in SQL (derived tables over `cargue_factura`); Rust reads the narrow rows
once and filters, sorts, pages (keyset cursor over (key, cargue, factura)), counts KPIs and facets in memory with the
Go semantics, with one implementation of the estado/pipeline/alert rules (`derive`) for list rows, aggregates and
the detail. Facets and totals need every row anyway, so SQL filtering would not avoid the scan. The cost is O(rows)
per request (tens of thousands of narrow rows); the 2.5 s shared answer cache bounds it. The cache seam is
`api::read::derive::cached`, to be pointed at `Cache::get_or_compute`. Known deviations: invoice names are stored
normalized (Go kept `fvs-3`, which sorted after `FVS7`); `actualizado_en`/`modificado_en` are RFC 3339 UTC.

### How a phase reads and updates invoices

```rust
use tessia::stores::dataset::DatasetError;

// Read-only phase (rips, soporte, derechos, ...): honour --invoices, take the header from the same read.
let subset = req.invoices.as_deref();                    // Option<&[String]>
let Some(ds) = ctx.dataset().read_current(&req.cargue_id, subset)? else { /* "no preparado" */ };
let max = ds.preparation.max_amount;                     // Option<Decimal>
let nt_version = &ds.preparation.nt_version;             // ask ctx.nt for rows of exactly this version
for inv in &ds.invoices { /* inv.lines, inv.version */ }
for ex in &ds.excluded { /* skip: reason ex.reason */ }
```

`update_invoices(cargue, &[(Invoice, expected_version)])` (Go `UpdateInvoices`): all-or-nothing; rows are
locked in name order and every invoice is checked first (missing or not-prepared -> `InvoiceNotFound`,
changed version -> `VersionConflict { invoice, stored, expected }`), nothing is written on failure; on
success lines, MIPRES ids and reconciliation are replaced, the position is kept and each version grows by
one. `Invoice::version` straight from the read is the expected version. Two invoices of different names never
conflict; `preparar` never conflicts (it replaces), but its `replace` bumps versions, so a `conciliar` that
read before it conflicts.

**Version-conflict pattern** (retry the whole read-compute-write unit, not just the write; database
transients such as deadlocks are already retried inside the store):

```rust
for attempt in 1..=3 {
    let mut invoices = store.read_invoices(cargue, Some(&names))?;
    let updates: Vec<(Invoice, i64)> = invoices.drain(..).map(|mut inv| { let v = inv.version; rebuild(&mut inv); (inv, v) }).collect();
    match store.update_invoices(cargue, &updates) {
        Ok(()) => break,
        Err(e) if e.is_retryable() && attempt < 3 => continue,   // re-prepared meanwhile: read again
        Err(e) => return Err(e.into()),                          // VersionConflict message is Spanish and user-facing
    }
}
```

If the expensive part is a network call (conciliar), do it once, then re-read only to detect a conflict: on
`VersionConflict` for an invoice that preparar re-prepared, Go fails the run with the conflict message
("vuelva a ejecutar el comando") instead of overwriting, which is also the right default here. `InvoiceNotFound`
is not retryable.

### Test datasets

```rust
use tessia::domain::dataset::test_support::{DatasetBuilder, InvoiceBuilder, sample_line};
use tessia::stores::dataset::test_support::seed_cargue;

let db = TestDb::new();
let ctx = AppContext::for_tests(&db);
let cargue = seed_cargue(&db.pool(), "Convenio_21102026_21102026", 'a'); // ingest into `cargues`, returns seq; another hash char = a newer upload
let ds = DatasetBuilder::new(cargue)                    // load defaults to Convenio_21102026_21102026
    .load("Convenio_21102026_21102026")                 // optional: set the load identity
    .nt_version("v1").max_amount("1500000".parse()?)    // optional header fields (also prepared_format, prepared_at, stats)
    .invoice(InvoiceBuilder::new("EVS1").line("10.50").line("5"))   // lines from sample_line(..): CC patient, 714 medicine, 1 unit
    .invoice(InvoiceBuilder::new("EVS2").with_line(my_line).mipres("11, 22"))
    .excluded("EVS3", "VALOR_SUPERA_MONTO")
    .manual_review("EVS4", "MULTIPLES_AFILIADOS_EN_FACTURA", "dos afiliados")
    .build();
ctx.dataset().replace(&ds, false)?;
```

`DatasetBuilder`/`InvoiceBuilder` are pure (no database) when you only need an in-memory `Dataset`; a phase's
own pure logic should take `&Dataset`/`&Invoice`/`&Line` so it is testable that way. Without the helper, build a
`Line` with `Line { plu: "x".into(), ..Line::default() }`. Both modules are behind `cfg(any(test,
feature = "test-support"))`, as `TestDb`. Integration tests of this store: `tests/dataset_store.rs`.

## How a phase is written

1. Work only inside `src/phases/<your_phase>/` (plus your migration band and, if needed, `config/keys.rs`
   entries, `Cargo.toml` dependencies, and `tests/<your_phase>_*.rs`).
2. Flags keep the Go names where they exist (`--invoices`, `--workers`, ...). No `--dry-run`, no
   `--live`, no confirmation prompts (S1).
3. In `run_phase`: load states -> gate -> `claim_available` -> `util::par::map` workers -> `save_outcome`
   -> report. Skip the invoices another machine owns and say so in `report.skipped`.
4. Per cargue work happens in `ctx.workdir(cargue)`; fetch inputs with `artifacts.fetch_*`, publish
   outputs with `artifacts.publish_*`. Nothing is read from or left on a fixed local folder.
5. Reasons/states/messages stay exactly as Go (they are a stored contract, S9).
6. Poll `tessia::shutdown::requested()` in long loops; stop starting work and let claims/workdirs clean up.

## Adding tables and migrations

* Migration directories: `migrations/<timestamp>_<name>/{up.sql,down.sql}`. Foundation uses
  `2026-10-22-0001xx..0008xx`. **Each phase has its own timestamp band** `2026-10-23-<NN>0000_...`:

  | NN | phase | NN | phase |
  |---|---|---|---|
  | 01 | convenio_descargar | 07 | cuv |
  | 02 | convenio_preparar | 08 | soporte |
  | 03 | thefactory | 09 | conciliar |
  | 04 | derechos | 10 | radicacion |
  | 05 | fev | 11 | proceso |
  | 06 | rips | | |

  e.g. `migrations/2026-10-23-020000_dataset_tables/`, next one `2026-10-23-020100_...`. Never edit a
  migration that was applied elsewhere; add a new one. `src/build.rs` re-embeds the directory
  automatically, so a new migration needs no manual rebuild step.
* Tables of a phase live next to its code, **not** in `src/stores/`: `src/phases/<phase>/schema.rs`
  (hand-written `diesel::table!`, like `src/stores/proceso/schema.rs`) and `store.rs`. `src/stores/mod.rs`
  and `src/db/` are shared files: do not edit them.
* Follow the store conventions in `src/stores/mod.rs` (idempotent statements via `pool.run`, units via
  `pool.transaction`, `NUMERIC` for money, database-clock timestamps, `ON CONFLICT` upserts, chunked
  `IN` lists with `stores::chunks`).
* Reuse, do not re-implement: `ProcesoStore` for phase state, `ValorXmlStore` for XML totals,
  `ParametrosStore` for parameters, `normalize_invoice` for every invoice key.

## Writing tests

```rust
use tessia::{context::AppContext, db::test_support::TestDb, nt::{MemoryNt, NtRow}, storage::MemoryStore,
             testing::{MockResponse, MockServer}};

let db = TestDb::new();                           // random database, migrated, dropped on Drop
let ctx = AppContext::for_tests(&db)              // MemoryStore + MemoryNt + private workdir root
    .with_nt(Arc::new(MemoryNt::new().with_version("v1", vec![("1".into(), NtRow::default())])));
let server = MockServer::start(|req| match req.path.as_str() {   // scripted HTTP server, records requests
    "/token" => MockResponse::json(200, r#"{"access_token":"t"}"#),
    _ => MockResponse::text(404, ""),
});
let client_base = server.url();                   // http://127.0.0.1:<port>
assert_eq!(server.requests()[0].header("authorization"), None);
```

* Unit tests (`#[cfg(test)]`) and integration tests (`tests/*.rs`) both can use `TestDb`,
  `AppContext::for_tests` and `MockServer` (integration tests get the `test-support` feature through the
  crate's self dev-dependency).
* `MockServer::sequence(vec![..])` answers in order (last repeats). Use `RetryPolicy::instant(n)` so
  retry tests do not sleep. Never contact Medicar, TheFactory, SISPRO, Mutual Ser or any real system.
* Redis tests: use a random key prefix and delete it afterwards (see `nt/redis_nt.rs`).
* Racing tests: use `std::sync::Barrier` and assert that only one owner wins (see
  `stores/proceso/tests.rs`).
* `tests/cli.rs` shows how to run the real binary (`env!("CARGO_BIN_EXE_facturacion")`) with an explicit
  environment.

## Conventions

* Code, comments, tests, commit messages in **English**; every user-facing string, state and reason in
  **Spanish** exactly as Go. Conventional Commits, no AI attribution.
* `println!` only for user output; diagnostics through `tracing` (stderr, `RUST_LOG`, default `warn`).
  Never log tokens, passwords, API keys, URLs with query strings or patient data.
* No `unwrap()`/`expect()` outside tests; errors are `anyhow::Result` at the edges and typed errors
  (`DbError`, `StorageError`, `NtError`, `HttpError`) in the libraries.
* No dry-run, no `--live`, no prompts (S1). Synchronous threads, no async runtime.
* Small functions; the only traits are `ObjectStore` and `NtCatalog` (they exist for tests and offline
  use). Money is always `Decimal`.
* Files are `0600` and workdir folders `0700`; the workdir deletes itself.
* Run before committing: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.

## Go behavior intentionally not ported

From S1/S2 (scope) and S3-S6 (design):

* Dry-run everywhere: `--live`, the `SI` confirmation, `internal/platform/dryrun`, `dryrun-out`, Guard lines.
* Commands out of scope: `serve` (HTTP API, CORS, dashboard, jobs, OpenAPI), `siesa *`, `migrate retire-log`,
  `seguimiento rebuild`, `convenio importar`, the desktop UI, the `version` command (use `--version`).
* MySQL machinery: DSN tuning, pools, `widths.go`, `lockretry.go` (replaced by `db::retry`), schema views,
  `LockConflictObserver`, MySQL `INSERT IGNORE`/`ON DUPLICATE KEY` (Postgres `ON CONFLICT`).
* File locks (`phaselock.go`, `internal/platform/lock`, `.tessia-*.lock`): replaced by `proceso_estado`
  claims and `leases`.
* Local working folders (`paths.automatic`, `paths.archivos`, `paths.siesa`, the `Radicado/` archive on
  disk): replaced by `Workdir` + bucket; the folder layout is recreated per cargue.
* Legacy INI config (`TESSIA_LEGACY_INI`), `siesa.*`, `serve.*` and `mysql.*` keys (kept only as retired
  names that a copied Go env file may still list).
* The xlsx/SQLite NT load and the `nota_tecnica_*` tables: the NT comes from Redis, read-only.
* `tessia.log` and the legacy log retirement; the live `proceso` panel and trace files.
* Not part of the foundation (owned by later tasks): TLS leaf pinning, and the `diario` (previous-day
  cron) entry point, which is new behavior (Wave 3). The dataset tables are the
  [Dataset store](#dataset-store) (task T1b).
