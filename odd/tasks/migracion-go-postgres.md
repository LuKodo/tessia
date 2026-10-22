# Feature: migracion-go-postgres
Objective: port a chosen subset of the Go CLI (`/Users/adatospharma/Desktop/TessiaAutomatic/tessia`, READ-ONLY reference) to Rust on Postgres + OCI bucket (Cassandra API) + Redis NT, runnable on N machines.
Branch: `feat/go-port-postgres` (baseline commit on `main`: SQLite NT-load CLI). Delivery: work-unit commits per phase branch, merged by the parent; no push/PR.
Test runner: `cargo test` (hermetic: mock HTTP servers, Postgres container `tessia-pg-test` on 127.0.0.1:55432, in-memory object store / NT fakes).

## Specs
- S1 (dry-run): "no vamos a poner modo dry-run, vamos directo a lo que es!" -> no dry-run, no `--live`, no `SI` confirmation anywhere.
- S2 (scope): "convenio descargar, convenio preparar, proceso run, conciliar, thefactory, derechos, fev, rips, cuv, soporte y radicacion." Sub-commands of those groups are in scope (`fev ajustar|verificar`, `cuv generar|barrer`, `radicacion run|recheck-step2`). Out of scope: `siesa`, `migrate`, `seguimiento`, `convenio importar`. (`serve`/`desktop` were out of scope until S10, which brings the dashboard + desktop window in.)
- S3 (multi machine): "esto deberia correr en n numero de maquinas, porque la idea es tener un cron corriendo automaticamente cada dia, haciendo el proceso completo del dia anterior. pero tambien hay que tener uno o dos pcs ejecutando el proceso "manual" y no deberian chocarse." -> cross-host coordination through Postgres (claims/leases), no `.lock` files; a "previous day" full-process entry point (`facturacion diario`) is a NEW behavior (Wave 3).
- S4 (database): "no pensemos en mysql sino en postgres. llevemos lo que ya habiamos pactado, pero usando postgres, bien ordenadito todo" -> Postgres via Diesel + embedded migrations; sync Rust (threads + blocking reqwest).
- S5 (files): "meterlos en la base de datos me parece mala idea ... creo que usando un oci bucket se podria ... cassandra, que permite subir, listar, descargar archivos de un bucket" and "en el bucket usa nombres planos, en la carpeta temporal, por necesidad puedes usar los _ y los prefijos." -> bucket key = `<cargue_id>__<relative path with "/" replaced by "__">`; Postgres `artefactos` index (never parse keys); per-machine temp workdir recreates the Go folder layout and is deleted after use.
- S6 (NT): "ya no necesitamos ni cargar ni copiar la nota tecnica, ahora debemos conectarnos a una coleccion de redis que estara siempre disponible con toda la informacion lista" -> read-only Redis client. Observed layout (read-only scan of local `nt-redis-test`): `nt:actual` (string = current version), `nt:versiones` (zset), `nt:<ver>:meta` (hash), `nt:<ver>:filas` (hash: field=`<id>` or `<id>#N`, value=JSON row with cum_facturar, cod_auf, cod_mutual, cod_gen_siesa, tipo_producto, ffm_rips, umm_rips, upr_rips, precio_contrato, precio_final, precio_maximo, subplan, contrato, clasificacion, estado, ...), `nt:<ver>:idx:<campo>:<valor>` (sets of ids; fields cod_auf, cod_mutual, cum_facturar, contrato, estado). The earlier xlsx/SQLite NT code is removed.
- S7 (fidelity): "migre, de la manera mas sencilla posible, el comportamiento de go a rust, todo el comportamiento!" -> faithful port of the Go behavior of the S2 commands (rules, states, reason strings, file names, formats), simplest implementation; deviations only those forced by S1/S3-S6 and recorded in the Log.
- S8 (assumption, not user-stated): cargue id = Go semantics (CSV stem, e.g. `Convenio_21102026_21102026`, `Manual_<ts>_<hash12>`), unique with CSV sha256, instead of the earlier timestamp+uuid id, because a re-run on any machine must resume the same cargue.
- S10 (visual part, 2026-10-22): "ahora falta una parte visual que emule lo que go hace en es ritorio" -> a desktop window that reproduces the Go desktop app (Wails window embedding the existing Lit/TS dashboard `../ui` with the API running in-process). DECIDED by the user ("usemos la c, con eso evitamos tener que rehacerlo", option C = web window + in-process custom protocol, existing UI reused untouched): reuse the existing UI instead of writing a new one; Rust shell with `wry`+`tao` serving the embedded UI and the API through a custom protocol (no TCP listener, like Wails). Remaining working assumptions (not user-stated): the API contract is Go's `docs/openapi.yaml`; the nota-tecnica upload route is replaced by a read-only Redis status; jobs run in-process, one at a time per machine.
- S9 (language): code/comments/tests/commit messages in English; user-facing messages, states and reasons in Spanish exactly as Go (they are a stored contract).

## Tasks
Wave 0 (parent, inline)
- T0 git baseline + branch + Postgres test container -- done, commit 17c8a9a (main)
Wave 1 (parallel)
- T1 foundation: restructure crate, remove SQLite/xlsx code, Postgres+Diesel+migrations, config, global stores (cargues, artefactos, proceso_estado claims, radicacion_estado, derechos cache, parametros, factura_valor_xml, leases), ObjectStore (Cassandra + memory) + Workdir, NtCatalog (Redis + memory), http helpers, Gson-compatible JSON, phase skeletons + CLI wiring -- route: delegated writer
- T2 fevxml: shared XML library (`src/fevxml`) used by thefactory/fev/cuv/conciliar/radicacion -- route: delegated writer (worktree)
Wave 2 (parallel, one worktree each; start after T1+T2 merged)
- T3 convenio descargar (Medicar client)
- T4 convenio preparar (CSV, NT cross, rules, MIPRES, dataset tables)
- T5 thefactory descargar (+CPScan)
- T6 derechos validar
- T7 fev ajustar/verificar
- T8 rips generar
- T9 cuv generar/barrer (+SISPRO client)
- T10 soporte generar (+PDF rendering)
- T11 conciliar (+Medicar formula client)
- T12 radicacion run/recheck-step2 (+Mutual Ser client, pipeline)
Wave 4 (S10, visual part)
- T15 explore UI + Go API contract (read-only) -> `odd/reference/ui-api-spec.md`
- T16 backend `api` module (routes + Postgres read models + in-process job runner)
- T17 desktop shell (wry/tao window, embedded UI build, custom protocol bridge)
- T18 UI adaptation (copy of `ui/` adjusted for the Rust backend, e.g. nota-tecnica panel)
Wave 3 (parent/writer, after Wave 2 merged)
- T13 proceso run (orchestrator, workers, replacement, radicacion batching) + `diario` (previous-day cron entry) + config check
- T14 integration pass: full `cargo test`, clippy, docs/help texts, README

## Log
- L1 (2026-10-22, user, verbatim excerpts): "no vamos a poner modo dry-run, vamos directo a lo que es!" / "convenio descargar, convenio preparar, proceso run, conciliar, thefactory, derechos, fev, rips, cuv, soporte y radicacion. seguimos con sqlite (verifica si en verdad es una ganacia usar mysql)" / "esto deberia correr en n numero de maquinas ... no pensemos en mysql sino en postgres." / "usando un oci bucket ... cassandra" / "en el bucket usa nombres planos ..." / "vamos, no te detengas a menos que requieras preguntar algo, paraleliza todo lo que puedas, tienes aprobacion total, asi que avaza"
- L2 (2026-10-22): Go behavior catalog (IDs CONV/PROC/CONC/TF/DER/FEV/RIPS/CUV/SOP/RAD) produced by two read-only explorers; docs: `<go>/docs/{convenio,proceso,conciliacion,thefactory,derechos,fev,rips,cuv,soporte,radicacion}.md` (some stale vs code: code wins).
- L3 (2026-10-22): Verified from Go history that MySQL deadlocks (1213) came from `tessia.log` (retired) and batch upserts; Postgres chosen anyway for N-machine sharing (S3). Cassandra API findings: reads unauthenticated (user's decision to harden), flat names only, `/files/search` is exact-name GetObject, same-name upload overwrites.
- L4: Engram mirror pending (mem_save hook failing: "Claude host session already ended"); local file is the source of truth.

## Progress
(updated per task)
- [x] T1 foundation (2026-10-22): crate restructured (lib `tessia` + bin `facturacion`), SQLite/xlsx code removed, Diesel/Postgres with 8 foundation migrations, config, db, domain, stores (cargues, artefactos, proceso claims, radicacion, derechos, parametros, valor_xml, leases), storage (Cassandra/memory + Artifacts + Workdir), nt (Redis/memory), http (retry, AIMD limiter, token cache), util (gson, fsx, decimal, text, par), AppContext, 11 phase skeletons wired in the CLI, docs/architecture.md. Observed: `cargo fmt` clean; `cargo clippy --all-targets -- -D warnings` clean; `cargo test` 195 passed (168 unit, 7 cli, 2 foundation, 18 doc) against TEST Postgres 55432 and TEST Redis 56379; `cargo run -- --help`, `config check`, `config check database.url` (exit 1, names the key and env var); `db migrate` on a throwaway database applied 8 migrations, a second run printed "la base de datos ya está al día" (8 rows in `__diesel_schema_migrations`), database dropped.

## Progress (parent log)
- 2026-10-22 T0 done. T1 done (10 commits, 195 tests). T2 done (fevxml, 30 tests) and merged (776adeb); merged tree: 198 unit tests green, clippy clean.
- 2026-10-22 Wave 2 launched: 11 writers in worktrees `.worktrees/<name>` (branch `feat/<name>`): t1b-dataset (dataset store, blocks Phase B of the others), convenio-descargar, convenio-preparar, thefactory, derechos, fev, rips, cuv, soporte, conciliar, radicacion. Two-phase protocol: writers finish Phase A, return `phase-a-done`, parent merges `feat/t1b-dataset` into `feat/go-port-postgres`, then messages each writer to run Phase B (cuv and convenio-descargar are fully independent).
- Behavior catalogs: `odd/reference/go-behavior-catalog-{a,b}.md` (generated by read-only explorers; Go source wins on conflict).
- Infra: test Postgres `tessia-pg-test` (127.0.0.1:55432, max_connections 800), test Redis `tessia-redis-test` (127.0.0.1:56379), shared cargo target `~/.cache/tessia-target`.
- Open follow-ups for Wave 3: unify Medicar login (descargar vs conciliar), unify NT->catalog mapping (preparar vs conciliar), `proceso run`, `diario`, `config check` review, help texts/README, remove `#![allow(dead_code)]` in fevxml.
- [x] T1b dataset store (2026-10-22): `domain::dataset` (Dataset/Preparation/Invoice/Line + test builders), `stores::dataset::DatasetStore` (replace/merge, subset reads, optimistic `update_invoices`, current-cargue pointer, `agreement_total`, `preparation`/`save_preparation`), migration `2026-10-23-000000_dataset_tables`, `ctx.dataset()`, docs/architecture.md "Dataset store"; 24 tests in `tests/dataset_store.rs`.

- 2026-10-22 L5 (user, verbatim): "ahora falta una parte visual que emule lo que go hace en es ritorio". Wave 4 opened. Wave 2 merged (all phases but proceso); real-data fixes by parent: NT concentration<-umm_rips, unit<-upr_rips, estado ERROR only ineligible. Radicacion merged; proceso writer (T13) still running.
- 2026-10-22 L6 (user, verbatim): "usemos la c, con eso evitamos tener que rehacerlo" (option C for the visual part). Verification wave launched (V1 concurrency, V2 security, V3a/V3b fidelity); T15 explorer running.
- 2026-10-22 L7 (session resume): previous session's agents found dead (0 background tasks); Wave 2/3 confirmed merged through proceso+diario (504cce8), spec docs committed (ff817d2 option C, a1ff938 ui-api-spec). Relaunched: T16 writer in `.worktrees/w4-core` (agent-0), T18 writer in `.worktrees/w4-ui` (agent-1), V1 concurrency / V2 security / V3 fidelity read-only reviewers (agent-2/3/4). T17 (desktop shell) pending T16, then parent merges + wires shared modules (cli.rs/main.rs/context.rs).
