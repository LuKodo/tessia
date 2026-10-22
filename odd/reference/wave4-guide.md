# Wave 4 writer guide (desktop dashboard: in-process API + shell + UI)

Read, in this order, before editing:
1. `odd/tasks/migracion-go-postgres.md` up to `## Log` (S9 and S10 bind this wave; the Log has the user's words).
2. `odd/reference/ui-api-spec.md` (the contract; read sections 2-7 fully, section 1 for the UI). Go source is READ-ONLY at `/Users/adatospharma/Desktop/TessiaAutomatic/tessia` and the UI at `/Users/adatospharma/Desktop/TessiaAutomatic/ui`; where the spec and the code disagree, the code wins and you report it.
3. `docs/phase-port-guide.md` "Hard rules", "Workspace and git" and "Verification" apply unchanged (no dry-run, no real external systems, hermetic tests, no `unwrap()`/`expect()` outside tests, English code/comments/commits, Spanish user-facing strings identical to Go, own migration band, shared cargo target dir, Conventional Commits with no AI attribution, never push or merge, never edit another writer's files).
4. `docs/architecture.md` for the foundation APIs (stores, `DbPool::run/transaction`, `AppContext`, `TestDb`, NT, storage).

## The skeleton you start from (commit `bd5b1e7`)
`src/api/{mod,request,errors,router,guard,cache,csv}.rs`, `src/api/read/*.rs` and `src/api/routes/*.rs` (every handler is a stub answering 501), `src/jobs/mod.rs` (placeholder `JobRunner`). The shared contract is fixed and you must not change it:
- `Request { method, path, query, headers (lower-case names), body, params }` and `Response { status, headers, body }` in `src/api/request.rs`; `Response::json`, `Response::json_status`; `ApiError::{new,bad_request,not_found,conflict,unavailable,internal,forbidden}`; `ApiResult = Result<Response, ApiError>`.
- Handler signature `fn(&ApiState, &Request) -> ApiResult`; `ApiState { ctx, pool (the API's own small DbPool), jobs, instance }`; path captures are in `req.params` in pattern order (`req.param(0)`).
- The route table and the handler names in `src/api/router.rs`. Replace the stub body of YOUR handlers only; keep names and signatures. A handler error is rendered by the router as `{"error": message}`.
- Parent-owned (do not edit): `src/cli.rs`, `src/main.rs`, `src/context.rs`, `src/lib.rs`, `src/shutdown.rs` unless your Allowed edit surfaces say so, `src/api/request.rs`, `src/api/errors.rs`, `src/api/router.rs` unless you own it. If you need a change there, report it.

## Ownership (each writer works in its own worktree and branch)
| Unit | Worktree / branch | Owns |
|---|---|---|
| A1 core | `.worktrees/w4-core` `feat/w4-core` | `src/api/{mod,router,guard,cache}.rs`, `src/api/routes/{health,nt,parametros,upload}.rs`, router/guard/cache tests |
| A2 read A | `.worktrees/w4-read-a` `feat/w4-read-a` | `src/api/read/{cargues,resumen,consulta,tracking,derive}.rs`, dataset summary columns migration + `DatasetStore` changes |
| B read B | `.worktrees/w4-read-b` `feat/w4-read-b` | `src/api/read/{informe,radicadas,portal,hallazgos}.rs`, `src/api/csv.rs`, `portal_hallazgo` migration + `PortalStore` |
| J jobs | `.worktrees/w4-jobs` `feat/w4-jobs` | `src/jobs/**`, `src/api/routes/jobs.rs`, the per-job cancel token |
| S shell | `.worktrees/w4-shell` `feat/w4-shell` | `src/desktop/**` |
| UI | `.worktrees/w4-ui` `feat/w4-ui` | `desktop-ui/**` |

Migration bands: A2 `migrations/2026-10-24-0000NN_*`, B `migrations/2026-10-24-0100NN_*`. A2 and B must not edit each other's tables.

## Shared rules for the API units
- Every handler reads through the API pool in `state.pool` (build stores with `Store::new(state.pool.clone())`), never through `state.ctx.pool`.
- Personal data: never expose `derechos_afiliado` (document numbers, birth dates) in any route; never log request bodies or query strings; error bodies use the generic messages the spec lists, never a raw database or network error.
- Field names, types, ordering, `null` vs `[]`, and error strings are copied verbatim from `ui/src/types/api.ts` and the Go code listed in the spec; arrays that Go serializes as `null` stay `null`.
- Tests go through the public interface, i.e. `ApiState::handle(Request)` against `TestDb`, the in-memory object store and `MemoryNt`; assert status, headers, body and persisted effects. Where the spec names a Go golden (`testdata/golden/dashboard/*.json`), build a synthetic equivalent and assert the same shape; never copy real patient or company data.
- `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (whole suite) must pass; a failing required command means `partial`/`blocked`.

## Return (max ~60 lines)
status; branch and commit hashes/subjects; spec items (`ui-api-spec.md` sections/routes) covered / deliberately deviated from / unverified; public API added; changes you need from parent-owned files (do NOT make them); verification results as `<command>: <observed result>`; open questions. Finish with `## Key Learnings` (1-5 standalone factual sentences of at least 4 words each).
