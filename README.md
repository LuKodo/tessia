# facturacion

Rust port of a subset of the Go `tessia` billing CLI. It downloads the daily Medicar convenio, prepares
invoices against the Nota Técnica (NT), runs the per-invoice phases (TheFactory, derechos, FEV, RIPS, CUV,
soporte, conciliación) and files them in Mutual Ser. It is designed to run on **N machines at once**
(a daily cron plus one or two manual PCs) without colliding.

* Database: **Postgres** (Diesel, embedded migrations). Shared state, claims and leases live here.
* Files (XML, PDF, ZIP, RIPS JSON, CUV TXT, CRC ZIP): an **OCI bucket** reached through the Cassandra storage
  API. Postgres keeps the index (`artefactos`); each machine works in a temporary directory that is deleted
  afterwards. Bucket keys are flat: `<cargue_id>__<relative path with "/" replaced by "__">`.
* Nota Técnica: a **read-only Redis** collection (`nt:actual`, `nt:<version>:filas`, `nt:<version>:idx:*`).
* Synchronous Rust (threads + blocking `reqwest`), no HTTP server, **no dry-run and no confirmation prompts**.

Architecture and module APIs: [`docs/architecture.md`](docs/architecture.md). How phases are written:
[`docs/phase-port-guide.md`](docs/phase-port-guide.md). Specs and decisions:
[`odd/tasks/migracion-go-postgres.md`](odd/tasks/migracion-go-postgres.md). Go behavior catalogs (IDs referenced
in code and docs): [`odd/reference/`](odd/reference/).

## Commands

```text
facturacion convenio descargar (--desde YYYY-MM-DD --hasta YYYY-MM-DD | --actual)
facturacion convenio preparar <CSV_O_CARGUE> [--monto-maximo N] [--invoices a,b] [--limit N]
facturacion proceso run <CARGUE> [flags]       # whole per-invoice pipeline + radicación batches
facturacion diario [--fecha YYYY-MM-DD]        # previous day end to end (cron entry point)
facturacion thefactory descargar --cargue ID
facturacion derechos validar --cargue ID
facturacion fev ajustar --op cucon|cuota --cargue ID   |   facturacion fev verificar --cargue ID
facturacion rips generar --cargue ID
facturacion cuv generar --cargue ID   |   facturacion cuv barrer --cargue ID
facturacion soporte generar --cargue ID [--regenerar]
facturacion conciliar --cargue ID
facturacion radicacion run <CARGUE> [--step 1-4 ...]   |   facturacion radicacion recheck-step2 --invoices a,b
facturacion config check [clave...]            # which keys are set (never prints values)
facturacion db migrate                         # idempotent, safe from several machines
```

Every command accepts `--config FILE` (a `KEY=VALUE` file; also `TESSIA_CONFIG`). `--help` on any command
shows the Spanish help.

## Configuration

Keys are dotted (`medicar.base_url`) and map to environment variables with the `TESSIA_` prefix
(`TESSIA_MEDICAR_BASE_URL`). Precedence, lowest to highest: built-in defaults, the `KEY=VALUE` file,
environment variables; empty values never override. Unknown keys in the file are an error. Run
`facturacion config check` to see every key and whether it is set. Core keys:

| Purpose | Keys |
|---|---|
| Postgres | `database.url`, `database.pool_size` |
| Nota Técnica (Redis) | `redis.url`, `redis.nt_prefix` (default `nt:`) |
| Files (Cassandra → OCI bucket) | `cassandra.base_url`, `cassandra.api_key`, `cassandra.bucket` |
| Local temp files | `workdir.root` (default `<os temp>/tessia-work`) |
| Company | `company.*`, `minsalud.*`, `zonaser.*`, `medicar.*`, `thefactory.*`, `cpscan.*`, `derechos.*`, `mipres.*`, `soporte.*` |

Secrets (passwords, tokens, API keys) are never printed or logged.

## Running on several machines

* Every phase claims work per `(invoice, phase)` in `proceso_estado`, so machines split a cargue's invoices
  instead of repeating them; a claim left by a crashed machine expires and can be taken over.
* Exclusive jobs use leases in Postgres instead of lock files: downloading a date range
  (`convenio_descargar:<cargue>`), per-invoice CUV/FEV exclusion (`cuv:<cargue>:<invoice>`), per-invoice
  soporte, and the single uploader per cargue in radicación (`radicacion:<cargue>`).
* Cron example (any machine, every morning): `facturacion diario` (previous local day, end to end).

## Development

```bash
docker run -d --name tessia-pg-test -e POSTGRES_PASSWORD=postgres -p 127.0.0.1:55432:5432 postgres:18
docker run -d --name tessia-redis-test -p 127.0.0.1:56379:6379 redis:7-alpine
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
```

Tests are hermetic: external systems are replaced by local mock servers and in-memory fakes; each test gets
a throwaway Postgres database (`TESSIA_TEST_PG_ADMIN_URL`, `TESSIA_TEST_REDIS_URL` override the defaults).
Nothing in the test suite contacts a real Medicar, TheFactory, CPScan, SISPRO, Mutual Ser or Cassandra.
