# Tessia UI

Lit web components + TypeScript single-page app (SPA) for the Tessia dashboard, built with Vite. It is a
separate project from the Go service: it only talks to the HTTP API of `tessia serve` and the Go
module never imports anything from `ui/`. It replaced the former Nuxt app (plan and history:
`../odd/tasks/ui-lit-migration.md`).

Routes: `/` (Dashboard: "Operar" and "Analizar" tabs), `/ejecuciones`, `/hallazgos`, `/parametros`,
`/revision-manual`.

Runtime dependency: `lit`. Dev tooling: Vite, vitest + happy-dom, TypeScript. No router, state, UI
or CSS framework: the components, tokens and the History API router are in `src/`.

## Run, build, test

Requires Node 24 and npm 11.

```sh
cd ui
npm ci             # or npm install
npm run dev        # http://localhost:3000
npm test           # vitest (unit tests, no backend needed)
npm run typecheck  # tsc --noEmit, covers src/ and tests/
npm run build      # static files in dist/ (index.html + assets/)
npm run preview    # serve dist/ locally
```

The build is a plain static site with a single-page-app shell: serve `dist/` with a fallback to
`index.html` for unknown paths (the Go desktop handler already does this).

## Environment

| Variable | Default | Meaning |
| --- | --- | --- |
| `VITE_API_BASE` | `http://localhost:8090` | Base URL of `tessia serve`. An explicitly empty value means same origin (`/api/*`, used by the desktop build). |

The value is baked in at build time (SPA): set it when running `npm run build`, for example
`VITE_API_BASE=http://tessia.lan:8090 npm run build`, or `VITE_API_BASE= npm run build` for same
origin (the bundle then contains no absolute API URL).

## Desktop window

The built UI is embedded in the native desktop executable. `scripts/build-desktop.sh` (repository
root) runs `npm ci` and the build with `VITE_API_BASE` set to the empty string, so the bundle calls
`/api/*` on the window's own origin and needs no CORS, then checks that `dist/` holds `index.html`
and no absolute API URL. The embedding step reads `desktop-ui/dist` when the executable is built.

## CORS requirement

In development the browser calls the API cross-origin (UI on `:3000`, API on `:8090`), so the service
must list the UI origin:

```sh
tessia serve --addr :8090 --allowed-origin http://localhost:3000
```

CORS is implemented in the Go core (`tessia/cmd/tessia/cmd_serve_cors.go`; the flag is repeatable,
config key `serve.allowed_origins` / `TESSIA_SERVE_ALLOWED_ORIGINS`, exact origin match, `*` is
rejected, none listed by default). Risk to keep in mind: the API has no authentication and listing
an origin also lifts the same-origin guard of the mutating routes for it, so any page served from a
listed origin can start and cancel jobs, upload files and write parameters. Only list origins you
control, on an internal network. See "Calling the API from another origin (CORS)" in
`tessia/docs/proceso.md`.

### Troubleshooting: "Sin conexión"

The navbar chip and the health cards say "Sin conexión" when `GET /api/health` gets no HTTP answer
(`ApiError` status 0). A browser cannot tell a blocked request from a stopped service, so the chip
tooltip and the health card list the configured API base and the page origin (see
`src/utils/connectivity-hint.ts`). Typical causes:

- The service is not running, or not listening at the address in `VITE_API_BASE`.
- `VITE_API_BASE` points to the wrong host or port. It is baked in at `npm run build`, so a built
  bundle needs a rebuild after changing it.
- The page origin is not listed in `--allowed-origin` (or `serve.allowed_origins` in `tessia.env`):
  the browser blocks the response (CORS) and reports the same network error. Origins match exactly,
  so `http://localhost:3000` and `http://127.0.0.1:3000` are different origins: list the one you
  type in the address bar.

An HTTP error (status 400 or higher) is not a connectivity problem: the chip shows "Degradado" or
the API error message instead.

## Layout and conventions

```text
src/
  main.ts            boot: tokens, pages, <tx-app>, saved theme
  config.ts, api.ts  API base from env, typed client (`api.cargues()`...)
  utils/, types/     framework-free models, the API client (`api-client.ts`, `tessia-api.ts`) and the
                     request/response types of every route (`types/api.ts`)
  router/            History API router + RouterController
  controllers/       Lit reactive controllers (PollController, JobPollController, dialogs, job alert,
                     hallazgos, parameters form, RIPS filters)
  theme/             light/dark choice persisted in localStorage
  styles/tokens.css  design tokens (CSS custom properties)
  components/        tx-* primitives (button, card, modal, select...), feature components and
                     rips/ (dashboard pieces), plus the icon set
  pages/             one element per route (page-*.ts, auto-registered by register-pages.ts)
tests/unit/          vitest + happy-dom
tests/support/       fixtures and fake API helpers
```

- Custom elements are prefixed `tx-` and use Shadow DOM; they style themselves from the tokens
  (`var(--tx-...)`), never with literal colors. Tokens are defined once on `:root` and inherited
  through shadow roots; light/dark follows `prefers-color-scheme` unless `data-theme` on `<html>`
  says otherwise.
- State and side effects live in reactive controllers (`host.addController`), not in elements.
  Remember `await el.updateComplete` in tests.
- Pure view logic goes in `src/utils/` with a test in `tests/unit/`.
- Icons are inline SVG (no icon font, no network): `<tx-icon name="...">` reads the bundled iconoir
  subset in `src/components/icons/icons.ts`; the app shell, tabs, timeline and modal close button
  still use the small stroke set of `src/components/icons.ts` (`icon(name)`).
- The logo comes from the opaque sources in `assets-src/`; `python3 scripts/make-logo-assets.py`
  (Pillow only) rebuilds `public/logo*.png`, `public/favicon.*` and the Windows `.ico`/macOS `.icns` used by
  the desktop build. The shell uses `logo-dark.png` (light wordmark) in the dark theme because the dark slate
  wordmark of `logo.png` is unreadable on the dark surfaces.
- Code, comments and tests are in English; UI copy is Spanish.
- Class fields rely on `useDefineForClassFields: false` (see `tsconfig.json`) so Lit reactive
  properties declared with `static properties` keep working.
- Adding a page: create `src/pages/page-<name>.ts` (a custom element, picked up by
  `register-pages.ts`), add its route (path, tag, label, icon; the sidebar is built from it) to
  `src/router/router.ts`, fetch with `api` from `src/api.ts` and poll with `PollController`.

## Relation to the Go service

- Routes and JSON shapes are mirrored in `src/types/api.ts`, derived from the Go structs
  (`tessia/cmd/tessia/cmd_serve*.go`, `tessia/internal/dashboard`; contract in
  `tessia/docs/openapi.yaml`). The UI follows the current contract; it does not change payloads.
- `src/utils/tessia-api.ts` has one typed method per route (`cargues()`, `jobsCurrent()`,
  `runJob(body)`, `saveParameters(body)`, ...); `src/api.ts` binds it to the configured base.
- Polling only (no SSE or websockets): `PollController` (interval may be a function of the latest
  data; interval ticks skip while a request is in flight and `refresh()` is latest-wins).
  `/api/jobs/current` goes through `JobPollController`: every 5 s while a job runs, every 2 s when
  idle.
- A job's `salida` is a short synthetic Spanish log (start line, one summary per finished phase, the
  first errors, the manual review pointer, the result), not a console stream. The verbose output
  lives in the terminal of `tessia serve` (see "Dashboard log vs the internal console" in
  `tessia/docs/proceso.md`).

### API contract notes

- Client errors answer 400, 404, 405, 409 or 503 with `{ "error": "..." }`; 500 is only for unclassified
  failures (see "API error conventions" in `tessia/docs/proceso.md`). Some older paths may still answer plain text.
- `getJSON` throws on a non-2xx and also on a 2xx body that carries `error`. `requestJSON` throws
  only on a non-2xx: use it for the portal endpoints, which answer 200 with a readable `error` next
  to usable data. Both throw `ApiError` (`status`, `message`, `body`); a network failure is an
  `ApiError` with status 0.
- `POST /api/run` and `POST /api/download` take capitalized keys (`Cargue`, `From`, `To`, `Limit`,
  `Desde`, `Hasta`); the other bodies are lowercase.
- Go nil slices arrive as `null` (e.g. `/api/jobs/history` before any job finished) and a zero
  `time.Time` is `"0001-01-01T00:00:00Z"` even with `omitempty`.
- A job snapshot of a failed or cancelled job carries `error` next to usable data, so
  `jobsCurrent()` and `retryInvoice()` are lenient (`requestJSON` semantics).
- Starting a job while another runs answers HTTP 409 `{ "error": "ya hay un proceso en curso" }`;
  the message is shown as is. `GET /api/jobs/current` always has `instancia_servidor`: a change while
  a job was running means the service restarted (the finish alert reports it).
- The manual review report CSV is a plain link (`informeCsvUrl()`, `?formato=csv`) on the API base;
  the server answers with `Content-Disposition: attachment`, so the browser downloads it even though
  the `download` attribute is ignored cross-origin.

## Dashboard > Analizar data flow

The "Analizar" tab does not download the table: the server filters, sorts, pages and aggregates
(`GET /api/seguimiento/consulta`, typed as `api.seguimientoConsulta(params)`; contract in
`tessia/docs/openapi.yaml`) and the UI renders what it answers.

- `RipsFiltersController` holds the draft form, the **applied** filters (they apply on Filtrar/Enter, a
  KPI click, a chart bar click or Limpiar) and the table sort. `query` is the filter + sort part of
  the request; it keeps its identity until they change, and `onQueryChange` fires then.
- `rips-dashboard` sends two requests together and repeats them every 10 s (`PollController`, paused
  while the tab is hidden):
  - the **rows**: `agregados=no`, the query, `limit=300` (`pageSize`);
  - the **aggregates**: the same query with `limit=0`, answering `total`, `kpis` (current month, or the
    whole cargue while a cargue filter is set; the server decides) and `facets` (estado options and
    alert counts).
- **Cargar más** appends the next page with `next_cursor`. Each poll refetches the loaded range from
  the start in one request (`limit` = rows loaded, at most 1000 per request, chained by cursor beyond
  that), so a refresh keeps the user's place. Changing the filters or the sort goes back to page one
  (a cursor is only valid for the sort it was issued for).
- `rips-table` shows the rows as they arrive (no client filtering, sorting or slicing), "N facturas"
  with the loaded count, the empty and error states, and emits `sort-change` and `cargar-mas`.
  `stabilizeRows` keeps the objects of unchanged rows so Lit skips them.
- List rows carry a short `estado_detalle` and no `lineas_convenio`; the detail modal fetches
  `GET /api/facturas/{factura}/seguimiento` on demand.
- The cargue chart still reads `GET /api/resumen` (every 30 s); its KPIs are not shown.
- Test helper: `tests/support/dashboard-consulta.ts` (`fakeConsulta`, `pageCalls`, `aggregateCalls`).

## Reportes (filed invoices)

Route `/reportes` (sidebar entry after "Revisión manual", `src/pages/page-reportes.ts`): every invoice
that is already *radicada* (all four filing steps confirmed, the same rule as the dashboard), with its
filing date and its observations. Filters: cargues (checkboxes; none checked means all), filing day
range (inclusive) and part of the invoice number. The table previews the first 500 rows ("Mostrando N
de M" when there are more); **Descargar CSV** is a plain link to the API with the same filters and
downloads every matching row, like the manual review CSV.

`GET /api/reportes/radicadas` (typed as `api.radicadas(query)` and `api.radicadasCsvUrl(query)`):

| Param | Meaning |
| --- | --- |
| `cargue` | repeatable; cargues to include (none = all). Unknown cargue: 404; invalid id: 400 |
| `desde`, `hasta` | `YYYY-MM-DD`, inclusive, compared with the filing day in the server's time zone; `hasta` before `desde` is a 400 |
| `q` | part of the invoice number (any case, dashes ignored; up to 60 characters) |
| `limite` | rows of the JSON preview (default 500, capped at 5000); ignored by the CSV |
| `formato` | `json` (default) or `csv` |

The JSON answer is `{ generado_en, total, truncado, filas }`; each row has `cargue`, `cargue_nombre`,
`factura`, `fecha_radicacion` (RFC 3339 UTC), `estado`, `fecha_factura`, `ssc`, `autorizacion`,
`valor` and `observaciones`. Rows are ordered newest filing first.

- `fecha_radicacion` is `radicacion_estado.updated_at` of the invoice: the pipeline writes the row
  when step 4 is confirmed and does not touch a finished invoice again.
- `observaciones` are the notes recorded for the invoice, oldest first, joined with ` | `: the reasons
  of its `proceso_estado` phases and `radicacion_estado.last_detail` (the legacy `tessia.log` notes
  were backfilled into it). Identical notes appear once.
- The CSV is named `radicadas-<todos|cargue|varios>-<YYYYMMDD>.csv`; UTF-8 BOM, `;` separator, CRLF
  (like the manual review CSV), a cell with `;`, quotes or line breaks is quoted. Columns: `Cargue`,
  `Nombre cargue`, `Factura`, `Fecha factura`, `Fecha radicacion` (local time, `YYYY-MM-DD HH:MM:SS`),
  `Estado`, `SSC`, `Autorizacion`, `Valor convenio`, `Observaciones`.
- Without a database nothing is filed, so the report is empty.
