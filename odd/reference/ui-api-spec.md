# Spec: Rust desktop dashboard (UI reuse, in-process API on Postgres)

Everything below was read directly from the files named. Items marked **(unverified)** were not provable from the read-only sources. All Go paths are relative to `/Users/adatospharma/Desktop/TessiaAutomatic/tessia`, all UI paths to `/Users/adatospharma/Desktop/TessiaAutomatic/ui`, and Rust paths to `/Users/adatospharma/Desktop/rust_version`.

## 1. UI

**Stack and build**
- Lit ^3.3.3 is the only runtime dependency. Dev tooling is TypeScript ~5.9.3, Vite ^8.3.4 and vitest ^5.0.4 with happy-dom (`package.json:11-21`).
- README:18 requires Node 24 and npm 11. Node is needed only to build the UI, not at runtime.
- There is no router, state or CSS framework. Components are `tx-*` custom elements using Shadow DOM. Routing is a History API router (`src/router/router.ts:15-22`).
- Build: `npm ci && VITE_API_BASE= npm run build` produces `dist/` (696K).
  - `dist/` holds `index.html`, `assets/index-*.js` (310 KB), `assets/index-*.css` (5 KB) and the favicon/logo PNGs.
  - Asset URLs are absolute (`/assets/...`) and the script is `<script type=module crossorigin>`.
  - Client routes (`/ejecuciones` and the others) need an SPA fallback to `index.html`.
  - `vite.config.ts` sets `outDir: dist`, `target: es2022` and a dev port of 3000.
- The existing `dist/` (built Oct 19) contains no `localhost:8090`, so it is already a same-origin build. `scripts/build-desktop.sh:41-47` greps for that string as a guard.

**API base**
- `src/config.ts:12` reads `import.meta.env.VITE_API_BASE ?? "http://localhost:8090"`.
- An empty value means same origin, and requests become relative `/api/...` (`src/utils/api-client.ts:25-32`).

**Desktop-specific code**
- There is none. No Wails bindings (`window.go`) and no feature flags exist in `src/`.
- Native `confirm()` and `alert()` are not used. `tx-app-dialogs` and `utils/dialogs.ts:21` replace them.
- localStorage is used for `tessia.theme` (`theme.ts:7`), so the origin must be stable.
- There are no external URLs, fonts or CDN references.
- Network-error text still mentions `tessia serve --allowed-origin` (`utils/connectivity-hint.ts`).
- The UI copy is Spanish with `lang="es"` and `es-CO` number/date formatting (`utils/format.ts:6`). The Parámetros page strings are in English. Code is English.

**Downloads and uploads**
- Download anchors:
  - Informe CSV: `revision-filtros.ts:179`, `download="informe-revision-manual-<id>.csv"`.
  - Radicadas CSV: `reportes-filtros.ts`, no `download` attribute, relies on `Content-Disposition`.
  - The router skips `download` anchors (`router.ts:87`). Other `/api/...` hrefs fall through to browser navigation, so the webview needs a download handler.
- Upload inputs are `<input type=file>` via `tx-file-upload`: the CSV in `nuevo-cargue.ts` and the XLSX in `nota-tecnica.ts`.

**Pages**

| Route | Shows |
|---|---|
| `/` | Dashboard with two tabs, mounted lazily. "Operar": health cards plus Nota técnica card, "Nuevo cargue" (Medicar date range download and manual CSV upload), "Ejecutar proceso" (cargue, from/to phase, limit), and "Registro del proceso" (live job card). "Analizar": monthly chart, 10 KPI cards, filters, a paged table, a detail modal and a portal modal. |
| `/ejecuciones` | Job history accordion. |
| `/revision-manual` | Manual-review items of all cargues, with single and batch retry of VALOR_DIFERENTE. |
| `/reportes` | Filed invoices (radicadas) with filters and CSV download. |
| `/hallazgos` | Findings catalog editor. |
| `/parametros` | Preparation parameters form plus the Nota técnica card. |

The shell also has a navbar health chip, a theme toggle and a job-finished snackbar.

**HTTP calls** (all from `src/utils/tessia-api.ts`)

"Strict" means `getJSON`, which throws on a non-2xx or on a 2xx body that carries `error`. "Lenient" means `requestJSON`, which throws only on a non-2xx. The UI never sends `If-None-Match` and never sets `Accept-Encoding`.

| Call | Caller and interval | Read mode |
|---|---|---|
| `GET /api/health` | indicator and cards, 5 s, paused when hidden | strict |
| `GET /api/nota-tecnica` | cards 60 s; Parámetros card on mount | strict |
| `GET /api/cargues` | proceso-panel 5 s; revision-manual and reportes on mount | strict |
| `GET /api/jobs/current` | console and snackbar, 5 s while running and 2 s idle, not paused when hidden | lenient |
| `GET /api/jobs/history` | historial 5 s | strict |
| `GET /api/seguimiento/consulta` | rows (`agregados=no`, `limit` 300, max 1000 chained by cursor) and aggregates (`limit=0`), both 10 s | strict |
| `GET /api/resumen?desde&hasta` | chart 30 s, current month | strict |
| `GET /api/facturas/{f}/seguimiento?cargue=` | detail modal, on open and on row change | strict |
| `GET /api/facturas/{f}/portal`, `POST .../portal/actualizar` | portal modal | lenient |
| `GET /api/cargues/{id}/informe` | revision-manual, one per cargue, plus a CSV link | strict |
| `GET /api/reportes/radicadas` | reportes on filter change, 300 ms debounce while typing; CSV is a link | strict |
| `GET /api/hallazgos`, `PUT /api/hallazgos/{c}` | hallazgos page | strict |
| `GET` and `PUT /api/parameters` | Parámetros page | strict |
| `POST /api/nota-tecnica` | multipart `workbook` | strict |
| `POST /api/convenio/upload` | multipart `csv` only | strict |
| `POST /api/download` | JSON `{Desde,Hasta}` | strict |
| `POST /api/run` | JSON `{Cargue,From,To,Limit}` | strict |
| `POST /api/cargues/{id}/reintentar` | JSON `{factura}` | lenient |
| `POST /api/jobs/cancel` | no body | strict |

Retry flow: it polls `jobs/current` every 1.2 s, then falls back to `jobs/history` (`utils/retry-model.ts`).

**Pieces tied to out-of-scope features**
- Siesa: only a confirm string, `proceso-panel.ts:130` ("Medicar, SISPRO, Siesa, MySQL, disco").
- MySQL: `health-model.ts:37` special-cases `nombre === "MySQL"` to show "ping".
- `paths.automatic` text: `proceso-panel.ts:176` and `nota-tecnica.ts:76`.
- Nota técnica upload: `components/nota-tecnica.ts`, mounted at `page-parametros.ts:192`. The status card is also in `health-cards.ts`.
- Nothing in the UI refers to `migrate`, `seguimiento rebuild` or `convenio importar`.

## 2. API contract

**Conventions**
- Every JSON answer has `Content-Type: application/json; charset=utf-8` and a trailing newline.
- Errors are `{"error":"..."}`. The status comes from `apiErrorStatus` (`cmd/tessia/serve_errors.go`): explicit status, otherwise 409 for the job-conflict errors, 503 while shutting down, otherwise 500.
- Router fallbacks answer 404 `ruta no encontrada` and 405 `método no permitido`. 405 also sets `Allow`.
- State-changing routes are behind the same-origin guard and answer 403 `solicitud de otro origen no permitida` (`serve_origin.go`).
- ETag, gzip and 304 apply only to `cargues`, `consulta` and `resumen`, with a 2.5 s shared cache (`serve_cache.go:22`, max 32 keys, only 200s cached). The desktop handler strips `If-None-Match`, `If-Modified-Since` and `Accept-Encoding` (`desktop_handler.go:45`), so in Rust the answers are always plain 200s.
- `Cache-Control: no-store` is set on `portal` and `hallazgos`.
- Body limits are 64 KiB for JSON (413 `cuerpo demasiado grande`, bad JSON 400 `cuerpo invalido: <err>`) and 32 MiB + 64 KiB for uploads.
- Go's JSON decoder matches keys case-insensitively. Rust must accept both `Cargue` and `cargue`, for example with `#[serde(alias)]`.
- Time is RFC 3339. A zero Go time serializes as `0001-01-01T00:00:00Z` where the field is a non-pointer time. Nil slices become `null`.
- The field schemas are in `ui/src/types/api.ts` and must be copied verbatim:

| Block | Lines |
|---|---|
| `Cargue` | 33-43 |
| `TrackingRow` | 89-123 |
| `ConsultaParams` | 136-163 |
| `Kpis` | 182-193 |
| `Summary` | 195-216 |
| Informe | 225-246 |
| Radicadas | 251-275 |
| Health | 293-301 |
| Nota técnica | 306-324 |
| Parameters | 329-342 |
| Hallazgo | 346-369 |
| Portal | 379-430 |
| Jobs | 434-537 |
| Upload | 541-550 |

Semantics that the types do not show are below.

**Routes**

| Route | Details |
|---|---|
| `GET /api/health` | Array in this order: `Medicar`, `SISPRO`, `ZonaSer · radicación`, `PortalMutualser · autorizaciones`, then the database (`MySQL` in Go). Each check is a concurrent TCP dial with a 3 s timeout, `latencia_ms`, and `detalle` on failure (`no configurado`, `URL invalida`, the dial error, or `una o más URL no disponibles`). Field names and rules are in `internal/dashboard/health.go`. |
| `GET /api/cargues` | Latest 20 cargues, `modificado_en` descending. `estado` ∈ COMPLETADO, CON_INCIDENCIAS, PROCESANDO, SIN_FACTURAS, SIN_DATOS, SIN_PREPARAR. Example: `{"id":"CargueA","nombre":"CargueA.csv","fecha_inicio":"2026-09-01","fecha_fin":"2026-09-25","facturas":7,"excluidas":1,"revision_manual":1,"modificado_en":"2026-09-20T08:00:00Z","estado":"PROCESANDO"}` |
| `GET /api/cargues/{id}/informe?formato=json\|csv` | 400 `formato debe ser json o csv`. 404 `dashboard: cargue not found: "<id>"`. Fields in `internal/dashboard/informe.go`; example `testdata/golden/dashboard/informe_a.json`. CSV is described below. |
| `POST /api/cargues/{id}/reintentar` | Body `{"factura":"x"}`, unknown fields rejected, 4 KiB limit. Errors: 400 `cuerpo JSON inválido`, 400 `factura requerida`, 404 not found, 503 `no se pudo verificar el estado persistido de la factura`, and 409 `la factura no tiene una revisión manual VALOR_DIFERENTE elegible`. Answers 200 with a `JobSnapshot`. |
| `GET /api/reportes/radicadas` | `cargue` repeatable (invalid 400 `cargue invalido "x"`, unknown 404). `desde` and `hasta` are validated as `YYYY-MM-DD` (400 `<name> debe ser YYYY-MM-DD`), and `hasta` before `desde` is 400 `hasta es anterior a desde`. `q` is at most 60 characters (400 `q no puede superar 60 caracteres`). `limite` is a positive integer (400 `limite debe ser un entero positivo`), default 500, capped at 5000, ignored by CSV. `formato` json or csv. Response `{generado_en,total,truncado,filas[]}`. |
| `GET /api/seguimiento/consulta` | Filters and rules are in section 3. Example of a row: `testdata/golden/dashboard/consulta.json`. 503 without a database. |
| `GET /api/resumen` | Default is the last 30 days ending today. Errors: 400 `hasta es anterior a desde`, 400 `el rango no puede superar 366 días`. |
| `GET /api/facturas/{f}/seguimiento?cargue=` | `{f}` is trimmed, upper-cased and stripped of `-`, then must match `^[A-Z]{3}\d+$` (400 `factura inválida`). Bad cargue is 400 `cargue inválido`. 404 `dashboard: invoice not found: "F"`. Without `cargue`, the newest cargue holding the invoice answers. The row has the full `estado_detalle` and `lineas_convenio[]`. |
| `GET /api/facturas/{f}/portal`, `POST .../portal/actualizar` | Order of checks: method, then 503 `la base de datos tessia no está configurada`, then 400 `factura inválida`. A portal failure is 200 with `error` set, otherwise 500 `no se pudo leer el resultado del portal`. The service rules (`internal/portalcargue/service.go`) are: refetch when forced, missing, or not final and older than 30 s (30 s timeout); `origen` is `base_de_datos` or `portal`; messages `El portal aún no tiene registro de esta factura`, `Portal de Mutual Ser no configurado`, `No se pudo consultar el portal: <scrubbed ≤200 chars>`. The scrub replaces URLs, bearer tokens, secrets and e-mails. |
| `GET /api/hallazgos` | `{hallazgos:[{codigo,detalle,genera_glosa,devolucion,descripcion}],total}` ordered by `codigo`. 503 `La base de datos no está configurada.` |
| `PUT /api/hallazgos/{codigo}` | Code must match `^[A-Za-z0-9_.-]{1,100}$` (400 `Código de hallazgo inválido.`). `genera_glosa` is required (400 `Falta el campo genera_glosa.`). `devolucion` is optional; absent or null keeps the stored value. `descripcion` is trimmed to at most 2000 chars (400 `La descripción no puede superar 2000 caracteres.`). Unknown fields or trailing JSON give 400. Unknown code 404 `El hallazgo no existe.` Store failure 500 `No se pudo acceder al catálogo de hallazgos.` |
| `GET /api/parameters` | `{configured,max_invoice_amount:"1500000",automatic_subplans[],wave_size,derechos_unavailable_percent,derechos_min_sample,cuv_token_failure_percent,cuv_min_sample}`. When nothing is saved the defaults are returned with `configured:false`. |
| `PUT /api/parameters` | Unknown fields or trailing JSON give 400. Validation is the one in `src/stores/parametros/mod.rs:73`, with Spanish messages. Go error bodies here are English or raw (`database is not configured` for 503). Returns 200 with `configured:true`. |
| `GET /api/nota-tecnica` | Go shape `{exists,file_name,size_bytes?,modified_at?}`. Redis adaptation is in section 7. |
| `POST /api/convenio/upload` | Multipart with exactly one `csv` field and a `.csv` extension. Errors: `Selecciona un único archivo CSV.`, `El CSV de convenio es inválido o no contiene registros válidos.`, `El CSV contiene filas con columnas incompletas o mal formadas.`, and 413 `El CSV supera el límite de 32 MiB.` Response `{"ok":true,"cargue":"Manual_<ts>_<hash12>","file_name":"<cargue>.csv"}`. The openapi `desde`/`hasta` fields are stale; the handler ignores them. |
| `POST /api/download` | Body `{"Desde","Hasta"}`. Errors: 400 `--desde must be YYYY-MM-DD`, 400 `--hasta must be YYYY-MM-DD`, 400 `--hasta is before --desde`. 409 `Ya existe un cargue para esas fechas; no se reemplazó.` when the cargue id `Convenio_ddMMyyyy_ddMMyyyy` already exists. Starts the job and answers 200 with a `JobSnapshot`. |
| `POST /api/run` | Body `{"Cargue","From","To","Limit"}`. Rules in section 4. |
| `GET /api/jobs/current` | `JobSnapshot`. When no job has run: `{"id":"","tipo":"","corriendo":false,"cancelable":false,"iniciado_en":"0001-01-01T00:00:00Z","terminado_en":"0001-01-01T00:00:00Z","salida":"","instancia_servidor":"9f2c..."}`. |
| `GET /api/jobs/history` | `null` before any job finished; otherwise entries as in section 4. |
| `POST /api/jobs/cancel` | `{"ok":true}`. 409 `no hay ningun proceso en curso`. |
| `GET /` | Plain text pointer. Optional; not needed in Rust. |

`JobSnapshot` omission rules (`serve_jobs.go:~470-505`):
- `terminado_en` is always present (zero value while running).
- `error`, `estadisticas`, `fases`, `radicadas` (0), `convenio` and `instancia_servidor` are omitted when empty.

Example snapshot:

```json
{"id":"job-3","tipo":"proceso run","corriendo":true,"cancelable":true,"iniciado_en":"2026-10-22T09:00:01-05:00","terminado_en":"0001-01-01T00:00:00Z","salida":"Inicio 09:00:01 · proceso run · cargue C1 · fases: ...","estadisticas":{"esperadas":10,"procesadas":10,"reemplazadas":0,"gestionadas":0,"terminadas":0,"con_errores":0,"excluidas":0,"revision_manual":0,"evaluadas":12,"limite":0},"fases":[{"fase":"derechos","etiqueta":"derechos validar","en_curso":2,"terminadas":8,"revision_manual":0,"excluidas":0,"con_errores":0,"tiempo_ms":12000}],"convenio":{"estado":"ok","evaluadas":12,"total":12,"preparadas":10,"excluidas":1,"revision_manual":1},"instancia_servidor":"9f2c0a"}
```

**CSV exports**
- Both exports use a UTF-8 BOM, `;` separator and CRLF line endings.
- A cell is quoted when it contains `;`, `"`, CR or LF, with quotes doubled.
- A cell starting with `=`, `+`, `-`, `@`, TAB or CR is prefixed with `'` (formula neutralization, `informe.go:csvCell`).
- Informe: `Content-Type: text/csv; charset=utf-8`, filename `Informe_revision_manual_<id with [^A-Za-z0-9._-]→_>.csv`, header `Factura;Fase;Categoria;Codigo;Motivo;Actualizado`. The category label is `Revisión manual`, `Falla` or `Excluida`.
- Radicadas: filename `radicadas-<todos|cargue|varios>-<YYYYMMDD>.csv`, header `Cargue;Nombre cargue;Factura;Fecha factura;Fecha radicacion;Estado;SSC;Autorizacion;Valor convenio;Observaciones`. The filing date is local time `YYYY-MM-DD HH:MM:SS`.

## 3. Read models and Rust mapping

**Rust sources**
- `cargues(seq,id,sha256,name,origin,date_from,date_to,created_at)`
- `dataset_preparacion(cargue_seq,load_id,load_name,load_from,load_to,prepared_at,updated_at,...)`
- `dataset_factura(cargue_seq,factura,factura_norm,estado PREPARADA|EXCLUIDA|REVISION_MANUAL,codigo,motivo,id_mipres,orden)`
- `dataset_linea(cargue_seq,factura,orden,total_value,data JSONB)`
- `dataset_actual(load_id,cargue_seq)`
- `proceso_estado(num_factura,fase,estado,motivo,updated_at,cargue_id)`
- `radicacion_estado(invoice,step 1-4,status,last_detail,updated_at)`
- `factura_valor_xml(num_factura,xml_valor,...)`
- `portal_cargue` and `portal_cargue_detalle`
- `parametros`

**Current cargue and listing**
- Current cargue of a load is `dataset_actual`, otherwise the newest upload (`DatasetStore::current_cargue`).
- Only current cargues are listed. A load with no `dataset_preparacion` is `SIN_PREPARAR`, built from `cargues`.
- `nombre` is `dataset_preparacion.load_name` (or `cargues.name`). Go's `modificado_en` is the preparation time, mapped to `dataset_preparacion.prepared_at` (or `cargues.created_at` when unprepared).
- `facturas`, `excluidas` and `revision_manual` are counts of `dataset_factura.estado`: PREPARADA, EXCLUIDA, and everything else.
- `id` is `cargues.id` and must be a single component without `/`, `\` or `..`. Rust cargue ids never contain `__`.

**Cargue status** (`internal/dashboard/dashboard.go` `statusFromCounts`)
- `SIN_DATOS` when there is no database.
- With no prepared invoices: `CON_INCIDENCIAS` if excluded + manual > 0, else `SIN_FACTURAS`.
- Otherwise count the derived estados of the prepared invoices (distinct names):
  - `terminados == total` gives `COMPLETADO`.
  - `terminados + RECHAZADO + VENCIDO + NOTA_AJUSTE == total` gives `CON_INCIDENCIAS`.
  - Anything else is `PROCESANDO`.

**Derived estado** (`native.go:Progress.Estado`), first match wins:
1. radicacion row step = 4 and status CONFIRMED gives `TERMINADO`.
2. status CREDIT_NOTE gives `VENCIDO` if `last_detail` upper-cased contains `RFFE022` (accented capitals folded), else `RECHAZADO`.
3. status ADJUSTMENT_NOTE gives `NOTA_AJUSTE`.
4. status BLOCKED with `last_detail` containing any of VALOR_FACTURA_DIFERENTE, VALOR_FACTURA_NO_DISPONIBLE, VALOR_CONVENIO_NO_DISPONIBLE or VALIDACION_VALOR_NO_DISPONIBLE gives `RECHAZADO`.
5. Any of `thefactory`, `rips`, `cuv` or `soporte` with state DONE gives `EN_PROCESO`.
6. Otherwise `CARGUE`.

Rust helpers: `phases/radicacion/rules.rs` has `expired_rejection` and `CODE_*` constants.

**Row fields**
- `artefactos[0..3]` are the phases `thefactory`, `rips`, `cuv`, `soporte`. DONE gives `ok`, FAILED or REVISION_MANUAL gives `error`, anything else `pendiente`.
- `radicacion[k]` for k = 1..4, from `radicacion_estado(step,status)`:
  - no row, or k > step: `pendiente`.
  - k < step, or status CONFIRMED: `ok`.
  - status in ADJUSTMENT_NOTE, CREDIT_NOTE, BLOCKED, REJECTED: `error`.
  - otherwise `en_curso`.
- `pipeline` (`summary.go:classify`), non-prepared rows are `no_aplica`. Prepared rows, first match:
  1. all four filing steps `ok` gives `radicada`.
  2. any filing step `error`, or `vencida_rffe022`, or estado RECHAZADO/VENCIDO gives `rechazada`.
  3. any filing step `ok` or `en_curso` gives `radicacion_parcial`.
  4. all four artifacts `ok` gives `lista_para_radicar`.
  5. any artifact `ok` or `error` gives `en_proceso`.
  6. otherwise `no_radicado`.
- `alertas`:
  - `falta_zip|json|cuv|crc` when the artifact is not `ok` AND the matching filing step is not `ok`. The pairs are zip↔step 1, json↔step 3, cuv↔step 2, crc↔step 4.
  - `pendiente_radicar` when the pipeline is not `radicada` or `rechazada`.
  - `nota_ajuste` when status is ADJUSTMENT_NOTE.
  - `nota_credito` when status is CREDIT_NOTE.
  - `requiere_autorizacion` and `sin_autorizacion` both when the authorization is blank (two codes, one rule).
  - `vencida_22_dias` when RFFE022 was matched.
- `requiere_nota_ajuste`, `requiere_nota_credito` and `vencida_rffe022` mirror the above. They are omitted when false.
- `estado` is set only for prepared rows that have native state; `estado_detalle` is described under "Detail text" below.
- `actualizado_en` is the maximum `updated_at` over the invoice's `radicacion_estado` row and all its `proceso_estado` rows. It is omitted when no row exists.
- Dataset-derived fields come from `dataset_linea.data` (snake_case `Line` keys), taking the first non-blank line value:
  - `fecha_factura` = first non-empty `date` cut to 10 characters.
  - `ssc`, `subplan` and `autorizacion` = first non-blank `ssc`, `subplan` and `authorization`.
  - `valor_convenio` = sum of `total_value` using `parse_agreement_amount`, with trailing zeros stripped. It is empty when there are no lines or any line is unparsable.
  - `id_mipres`: blank subplan gives `""`; subplan normalized (accents, case, spaces) equal to `EVENTO NO PBS` gives `dataset_factura.id_mipres`; any other known subplan gives `No aplica`.
  - `fecha_inicio_cargue` and `fecha_fin_cargue` are `dataset_preparacion.load_from` and `load_to`.
- `valor_xml` = `factura_valor_xml.xml_valor`, plain decimal with trailing zeros stripped. The Rust thefactory and fev phases already write it. Do not read the bucket on the read path.
- `portal_*`: `portal_consultado` is true when a `portal_cargue` row exists. `portal_estado` = `estado`, `portal_advertencias` = `total_advertencias`, `portal_errores` = `total_errores`, `portal_final` = `es_final`.
- Detail text:
  - Go takes `radicacion_estado.last_detail` (whitespace collapsed, capped at 600 runes) when non-blank, otherwise the reason of the most recently updated `proceso_estado` row (ties go to the later row).
  - Excluded and manual-review rows use the dataset reason; manual-review text is `codigo: motivo`.
  - List rows cut to 90 runes plus `…`; the detail route returns the whole text.
  - Override: if the detail contains `VALOR_FACTURA_DIFERENTE` and the `conciliacion` state is EXCLUDED with code `BLOQUEADA_TARIFA` or `BLOQUEADA_MONTO`, the text becomes:
    - `Bloqueada por tarifa: un precio del XML supera la nota técnica. Requiere nota crédito y refacturación.` or
    - `Bloqueada por monto: el total reconstruido supera el máximo permitido por factura.`

**`/api/seguimiento/consulta`**
- Rows are the `dataset_factura` rows of every current cargue.
- Text filters `factura`, `cargue` (matches `load_name + " " + load_id`), `ssc`, `autorizacion`, `subplan` and `mipres` (matches the computed `id_mipres` text) are:
  - trimmed, lower-cased, at most 200 runes (400 `<name> es demasiado largo (maximo 200 caracteres)`),
  - substring tests, accent-sensitive.
- `estado` is exact against the display estado: estado upper-cased, or `SIN INICIAR` for a prepared row with no native state, `EXCLUIDA`, or `REVISION_MANUAL`.
- `pipeline` is exact (400 `pipeline desconocido "x"`). `alerta` is a comma list or repeated parameter with OR semantics (400 `alerta desconocida "x"`).
- `desde` and `hasta` are `YYYY-MM-DD`, compared as text on `fecha_factura`, and a row with no date is excluded if either is set.
- Excluded and manual-review rows appear only when `pipeline` or `estado` is set.
- Sort: no `sort` means (cargue, factura) ascending. `sort` ∈ `fecha_factura`, `actualizado_en`, `factura`, default `dir=desc` (`dir` invalid 400 `dir debe ser asc o desc`). Ties break by (cargue, factura) ascending.
- Paging: `limit` 0..1000, default 300. `limit=0` with `agregados=no` is 400 `limit=0 solo pide los agregados: no puede combinarse con agregados=no`. `agregados` ∈ `si|no`.
- The cursor is `base64url(JSON{k,c,f,s,d})`. A cursor for a different sort is 400 `cursor no corresponde al orden pedido`; a malformed one is 400 `cursor invalido`.
- `total` counts rows matching the filters.
- `kpis` count the prepared rows that match the filters and have an invoice date inside `kpi_desde..kpi_hasta`.
  - The default bounds are the first and last day of the current month.
  - The bound is lifted while the `cargue` filter is set.
  - The date test is text: a missing date fails a lower bound but passes an upper one.
  - Fields: `total`; `no_radicado`, `en_proceso`, `listas_para_radicar`, `radicacion_parcial`, `radicadas`, `rechazadas` by pipeline; `nota_ajuste`, `nota_credito` and `sin_autorizacion` from the alert flags.
- `facets.estados` is the sorted distinct display estados over every row, ignoring filters. `facets.alertas` is the count per alert over every row, ignoring filters, non-zero only.
- `next_cursor` is null on the last page.
- Rust gap: `dataset_factura` has no stored date, SSC, subplan, authorization or total columns. See section 7.

**`/api/resumen`**
- `kpis` use the same row rules, with the invoice date in `[desde, hasta]` inclusive as text.
- `cargues[]` has one entry per current cargue with at least one invoice, ignoring the dates:
  - `total` = every invoice row; `gestionadas` = prepared; `excluidas` = EXCLUIDA; `revision_manual` = everything else.
  - `radicadas` = prepared rows with pipeline `radicada`.
  - `valor_cargado` = sum of `valor_convenio` over prepared rows.
  - `valor_radicado` = the same sum over filed rows.
  - Both are plain decimals.
- Sort: newest `fecha_inicio` first, undated last, ties by `cargue` descending.

**Informe**
- Items are drawn from the dataset and from `proceso_estado`.
- For each non-DONE and non-EN_PROGRESO row of the phases `convenio, derechos, thefactory, fev_ajuste, fev_valor, conciliacion, rips, cuv, soporte`:
  - categories: REVISION_MANUAL gives `revision_manual`, FAILED gives `falla`, EXCLUDED gives `excluida`.
  - `codigo` is the leading `CODE: ` of the reason (regex `^([A-Za-z][A-Za-z0-9_]*):\s+(.+)$`), else empty.
- Dataset excluded and manual-review entries are items of phase `convenio` and are dropped if a `convenio` state is DONE.
- An `fev_valor` item is dropped when `conciliacion` is EXCLUDED with a `BLOQUEADA_*` code updated at or after it.
- Sort: category (manual, fail, excluded), then phase order, then invoice.
- `incompleto` is true only when states could not be read.

**Radicadas** (`internal/dashboard/radicadas.go`)
- Rows are prepared rows with pipeline `radicada`.
- `fecha_radicacion` = `radicacion_estado.updated_at` of the filed invoice. The `desde`/`hasta` filter is applied to the local-time day of that date.
- `q` is a substring match on the normalized invoice.
- `observaciones` are the distinct notes (each `proceso_estado.motivo` plus `radicacion_estado.last_detail`, whitespace collapsed), oldest first, joined with ` | `.
- Order: newest filing first, ties by cargue then invoice.
- `total` is the count before `limite`, and `truncado` says whether `filas` has fewer rows.

**Portal view mapping**
- Portal view fields map from `portal_cargue`: `consultado` = row exists; `final` = `es_final`; `estado`, `remision`, `fecha_cargue_texto`, `usuario`, `servicios`, `modalidad`, `nombre_archivo`, `id_archivo`, `id_cargue`, `id_evento`, `tipo_evento`, `tipo_error`, `codigo_error`, `devuelto`, `fecha_expedicion`, `receipt_name` same-named.
- `fecha_cargue` = `fecha_cargue` formatted RFC 3339 in UTC-5.
- `documento_soporte` = the stored JSON when it is an object, else `null`.
- `advertencias`, `errores` and `consultas` come from `total_advertencias`, `total_errores` and `consultas`; `consultado_en` = `consultado_en` in UTC.
- `hallazgos[]` = `portal_cargue_detalle` rows by `orden` mapped to `{codigo,tipo,campo,detalle}`.
- `origen`, `mensaje` and `error` come from the lookup logic above.
- Rust building blocks: `phases/radicacion/portal.rs` has `PortalStore::get` and `Follower`.

## 4. Jobs

**Runner** (`cmd/tessia/serve_jobs.go`)
- Exactly one job runs at a time per process. A second start is 409 `ya hay un proceso en curso (<tipo>, iniciado HH:MM:SS): espera a que termine`.
- Ids are `job-N`. The sequence continues after the highest persisted id.
- Each job runs in its own goroutine with live mode, no prompts and an empty stdin.
- Verbose output goes to the process console. The API exposes only the synthetic log `salida` (`synthetic_log.go`):
  - start line `Inicio HH:MM:SS · <tipo> · cargue <id> · fases: a → b`,
  - convenio result line,
  - when over: one line per phase `<Etiqueta>: N ok · N revisión manual · N con errores[ · N excluidas] · <t>`,
  - up to 5 error lines `Error · <factura> · <fase> · <reason ≤160>` then `+N errores más`,
  - the manual-review pointer,
  - a result line `Proceso terminado en <t>: ...`, `Proceso con error tras <t>: <err ≤240>`, or `Proceso cancelado por el operador tras <t>`.
  - Elapsed format is `45 s`, `3 min 12 s`, `1 h 05 min`.
- A panic becomes the job error `panic: <value>` and is recorded.
- Cancel sets the error `cancelado por el operador`. The UI also matches `cancelad[oa]|cancell?ed`.
- Shutdown rejects new starts with 503 and cancels and waits for the current job.
- The UI detects a restart by a change in `instancia_servidor` (random hex per process).
- Event types that feed `estadisticas`, `fases` and `convenio`: `convenio_start`, `convenio_progress`, `convenio`, `convenio_more`, `invoice_replace`, `run_start`, `invoice_enter`, `invoice_step`, `radicacion_enter`, `radicacion`, `run_end`. Statuses are `OK`, `MANUAL_REVIEW`, `EXCLUDED`, `FAILED`.
- `fases[].fase` keys are `derechos, thefactory, fev, rips, cuv, soporte, conciliar, radicacion`. The UI names them in `utils/phase-progress.ts:51-60`. `/api/run` seeds all planned phases at zero, in plan order, and a phase that never ran is dropped once the run ends.
- Elapsed time per phase is busy wall time (at least one invoice in flight). Radicación time is the sum of its batch send durations.

**History**
- File `<paths.automatic>/.tessia-serve-jobs.jsonl`, one JSON per finished job, append-only, never trimmed, mode 0600.
- Line shape: `{"id":"job-1","tipo":"proceso run","iniciado_en":"...","terminado_en":"...","error":"...","salida_final":"<last 2048 bytes>","estadisticas":{...}}`. `error`, `salida_final` and `estadisticas` are omitted when empty.
- At start the last 50 entries are loaded newest first, unparsable lines are skipped, and the scan allows lines up to 1 MiB.
- `/api/jobs/history` returns those in memory, newest finished first.

**Job types and validation**

| `tipo` | Command | Validation order |
|---|---|---|
| `convenio descargar` | `--desde --hasta --out <folder>`. The folder `Convenio_ddMMyyyy_ddMMyyyy` is created first and released if the job cannot start or fails with an empty folder. | method; body; `parseDay` dates; hasta ≥ desde; folder exists then 409; start. |
| `proceso run` | `[--from F] [--to T] --limit N <cargue dir>` | body; `ValidCargueID` else 400 `cargue invalido "x"`; `Limit<0` else 400 `--limit must be >= 0`; cargue exists else 404 `dashboard: cargue not found: "x"`; then start. |
| `conciliar invoice <FACTURA>` | `--invoices F <cargue dir>` | the retry rules in section 2. |

- `From` and `To` use the Go step names `convenio, thefactory, fev, conciliar, derechos, rips, cuv, soporte, radicacion` (`cmd_proceso.go:125`). They are not validated when the request arrives. An unknown name or `from` after `to` fails inside the job with `unknown step "x" (available: ...)` or `--from "x" is after --to "y"`. Rust may reject these up front with 400.
- `Limit` counts prepared invoices only; excluded ones are replaced by the next candidates. A `SIN_PREPARAR` cargue is prepared first (step 0).
- `fev` maps to `fev_ajuste` plus `fev_valor`, and `conciliar` maps to the Rust phase `conciliacion`.

**Folder-dependent UI**
- Folder-dependent UI is the `paths.automatic` placeholders (`proceso-panel.ts:176`, `nota-tecnica.ts:76`), the `Convenio_*` folder claim on download, and the flat id `.`.
- Replace them with `cargues` rows in the database. The "no cargues" placeholder is just `sin cargues`.
- Download duplicate detection becomes `CarguesStore.newest(id)` for 409. The download lease (`convenio_descargar`) already blocks another machine from downloading the same range.
- Keep the `.tessia-serve-jobs.jsonl` format, but store it under the OS user data directory instead of `paths.automatic` (history is per machine, as in Go). A Postgres table would be a design change.

## 5. Desktop (what Wails gives and what wry/tao must reproduce)

- Window: title `Tessia`, 1280x800, min 960x600 (`desktop_window.go:49-57`). Everything else is Wails v2 defaults, which Go does not set explicitly **(unverified)**: no frameless, no menu, system theme, default context menu behavior.
- Assets: `newDesktopHandler(ui, api)` routes `/api` or `/api/*` to the API handler in the same process and everything else to the static UI. `staticUIHandler` (`desktop_handler.go:70`) serves:
  - exact files, and `dir/index.html`,
  - a missing file with an extension is a real 404,
  - any extension-less unknown path gets `200.html` or else `index.html` (SPA fallback),
  - non-GET/HEAD on static is 405 with `Allow: GET, HEAD`.
- No TCP listener, no CORS, no `serve` command. The UI is embedded at build time (`go:embed`, `requireUIIndex`). If `index.html` is missing the startup error is `this build embeds no UI (index.html not found); build it with tessia/scripts/build-desktop.sh`.
- Startup order: embedded UI check, then open stores and job runner (`openServeBackend`), then open the window. Closing the window stops the runner and releases the stores (`defer backend.Close()` cancels the running job).
- `defaultArgs` (`main.go:32`, `desktop_window.go:29`): with no arguments the desktop build runs `desktop`. A double-click therefore opens the window. Global flags (`--config`) apply. `TESSIA_CONFIG` points at the config file when there are no flags.
- Failure dialog: the Windows build is `-H windowsgui` and has no console. `runReporting` (`failure_report.go`) captures the last 1500 bytes of stderr and, on any non-OK and non-130 exit, calls `showFailureDialog`, a `MessageBoxW` with title `Tessia` and fallback text `Tessia could not start.` macOS shows no dialog.
- Request handling:
  - `desktopAPIRequest` drops `If-None-Match`, `If-Modified-Since` and `Accept-Encoding`.
  - It marks `Origin` of `http://wails.localhost` (Windows) or `wails://wails` (macOS/Linux) as allowed. Other origins stay subject to the guard.
  - Wails turns a 304 into a 500 on Windows, so the API must always answer a plain 200.
- Downloads and save dialogs: Go has no download or save-dialog code. How Wails delivers `Content-Disposition: attachment` responses is **(unverified)**. For wry:
  - Use the download-started handler to pick a path (the OS Downloads folder, optionally with a native save dialog).
  - Use the download-completed handler to notify.
  - Check this against the pinned wry version; I read no wry documentation.
- Custom protocol on wry: origin is `<scheme>://localhost` on macOS/Linux and `http://<scheme>.localhost` on Windows **(unverified)**.
  - Use the asynchronous protocol variant with a worker pool. The handler runs on the UI thread and blocking Diesel or HTTP calls would freeze the window **(unverified)**.
  - The Origin header on POST may not be forwarded as in Wails **(unverified)**. See section 6.
- File chooser for `<input type=file>` (the CSV upload) in wry on macOS/Linux is **(unverified)**.

## 6. Security

**Made unnecessary by in-process custom protocol**
- CORS middleware, preflight handling, `--allowed-origin`, `serve.allowed_origins`, and `Access-Control-*` headers.
- TCP listener and server timeouts (slowloris protections, `newServer`), the `--obs-addr` pprof server, loopback Host checks, and `GET /`.
- ETag, If-None-Match, 304 and gzip. The desktop handler already strips them.
- The `connectivity-hint` text about `--allowed-origin`.
- Remote LAN callers (the Go `serve` allowed anyone on the LAN).

**Must be kept**
- The API has no authentication, so reachability must stay in-process only. Block navigation of the webview away from the app origin and block new windows (wry navigation and new-window handlers **(unverified)**).
- Keep the same-origin guard semantics (`serve_origin.go:21`) on every state-changing route: reject cross-site (`Sec-Fetch-Site: cross-site`) and any `Origin` that differs from the app origin; allow requests with neither. A custom scheme can be reached by other pages loaded in the webview, so do not trust the origin blindly. Treat requests from the app origin as trusted regardless of how Origin is forwarded.
- Personal and business data in responses: invoice numbers, SSC, authorizations, MIPRES ids, portal `usuario` and the raw `documento_soporte`, patient-related lines.
  - Keep `derechos_afiliado` (document numbers, birth dates) out of every route.
  - Never log request bodies or query strings.
  - Disable devtools in release.
  - Keep `Cache-Control: no-store` on portal and hallazgos. Consider `no-store` on all API answers so personal data does not land in the webview disk cache.
- Error hygiene: map store and network errors to generic messages as Go does (`no se pudo leer el resultado del portal`, `No se pudo acceder al catálogo de hallazgos.`), and port `portalError` scrubbing (URLs, bearer, secrets, e-mails, 200 chars).
- Input validation to port: `ValidCargueID`, invoice `^[A-Z]{3}\d+$`, hallazgo code regexp, text length caps, body size caps (64 KiB JSON, 4 KiB retry, 32 MiB uploads), and the CSV formula neutralization in `csvCell`.
- File uploads: validate the `.csv` extension and every row before ingesting.

## 7. Risks and gaps

**Rust data gaps**
1. `portal_hallazgo` is MISSING in Rust (no table, no store; `PortalStore::save` registers no findings). Needs a migration, a seed from `portal_cargue_detalle`, `INSERT ... ON CONFLICT DO NOTHING` on each save (first detail wins), plus list and update.
2. Stored summary columns (date, ssc, subplan, authorization, total) are MISSING on `dataset_factura`. Go filters, sorts and aggregates over such columns in SQL (`cargue_factura.fecha_factura_txt`, `ssc`, `subplan`, `autorizacion`, `valor_convenio`). Options: add the columns in a migration and fill them in `DatasetStore::replace`/`update_invoices` plus a backfill (recommended, mirrors Go), or derive from `dataset_linea.data` JSON (slow and cannot parse `1.234,56` in SQL). The `valor_convenio` sum needs `parse_agreement_amount`.
3. There is no persisted job history; use the JSONL file (section 4). The Rust job runner and its event stream do not exist, and `proceso run` is still a skeleton (`src/phases/proceso/mod.rs`, `bail!("not implemented yet")`). T13 must emit the event types listed in section 4 and support `--from`, `--to` and `--limit` with replacement semantics. `proceso run` has no `--limit`, `--from` or `--to` yet.
4. Job cancel: `src/shutdown.rs` is a global one-way flag with no reset. A UI cancel needs a per-job cancellation token or a resettable flag that the phases poll. This is a shared-file change.
5. `GET /api/health` needs Rust names: the Postgres check (the UI hard-codes the name `MySQL` for "ping"); optional rows for Redis (NT) and the Cassandra bucket; the other four services read `medicar.base_url`, `minsalud.base_url`, `zonaser.login_url` + `zonaser.benavente_base_url`, and `portalmutualser.base_url`.
6. Manual CSV upload has no Rust producer. A `Manual_<ts>_<hash12>` cargue needs `convenio_preparar::csv::read_csv` for validation (reject empty or skipped rows), `CarguesStore::ingest` with origin `manual`, and publishing the CSV to the bucket.
7. `valor_xml` is only as fresh as the thefactory/fev phases that populate `factura_valor_xml`. Go refreshed it lazily from the files.
8. SQL parity for the filter, sort, cursor, and KPI rules should be proven against `testdata/golden/dashboard/*.json` through a parity test.

**UI changes**
- Replace the Nota técnica card with a read-only view of the Redis NT and drop the upload route. Suggested response: `{"exists":true,"file_name":"<version>","version":"<version>","filas":"<meta.filas>"}`; the `meta` hash keys are unknown, only `filas` and `fuente` appear in test fixtures (`src/nt/redis_nt.rs:219-220`). `health-cards` and `notaTecnicaDetail` currently read `size_bytes` and `modified_at`, and `nota-tecnica.ts` writes about `paths.automatic`.
- Edit the `proceso-panel.ts` confirm text (drop Siesa and MySQL), the `sin cargues en paths.automatic` placeholder, the `MySQL` special case in `health-model.ts:37`, and the connectivity hint text.
- Decide whether the manual CSV upload card stays (Rust needs gap 6).
- The `wave_size` field stays (legacy, still stored). The `proceso run` `From`/`To` options need the phase-name mapping from section 4.

**Other risks**
- Wry limitations listed as unverified in section 5 (async protocol, download handlers, file chooser, Origin forwarding, navigation blocking).
- Rust has no `modificado_en`; use `prepared_at` as in Go.
- The openapi spec drifts from the code in the upload fields; the code wins (`serve_openapi_test.go` checks routes only).
- A running job holds DB connections. Give the API its own small `DbPool`, as Go does (API pool of 20, jobs on the process pool).
- Polling load: about 12 requests per 5 s across health (×2), jobs/current (×2), cargues, historial, consulta (×2) and resumen. Keep the 2.5 s shared cache for `cargues`, `consulta` and `resumen`, and cap health checks at 3 s.
- Daily quirks to preserve: nil slices as `null` on `jobs/history`, capitalized request keys on `run` and `download`, `terminado_en` always present, `salida_final` cut at 2048 bytes (cut on a character boundary in Rust).

## Proposed split of work

1. **API core and wiring** (`src/api/{mod,router,errors,guard,cache,csv,json}.rs`)
   - Routing, JSON error bodies, 404/405 fallbacks, same-origin guard, TTL cache with request coalescing, CSV writer, case-insensitive request keys, body limits, `Cache-Control` rules.
2. **Read models A: cargues, resumen, consulta, factura** (`src/api/read/{cargues,resumen,consulta,tracking,derive}.rs` and migration `migrations/2026-10-23-12xxxx_dataset_summary_columns`)
   - Estado/pipeline/alert derivation, SQL filters, cursor, KPIs, facets, summary columns migration, backfill and `DatasetStore` changes.
3. **Read models B: informe, radicadas, portal, hallazgos** (`src/api/read/{informe,radicadas,portal,hallazgos}.rs`, migration for `portal_hallazgo`, change to `PortalStore::save` in `src/phases/radicacion/portal.rs`)
4. **Parameters, health, NT status, uploads** (`src/api/routes/{parametros,health,nt,upload}.rs`)
   - Manual CSV upload and the Postgres/Redis/bucket/HTTP health checks.
5. **Job runner and job routes** (`src/jobs/{mod,runner,snapshot,synthetic_log,history}.rs`, routes `run`, `download`, `reintentar`, `jobs/*`)
   - One job at a time, event observer, cancel token, JSONL history. Depends on T13 events and on the `shutdown.rs` cancel change.
6. **Desktop shell** (`src/desktop/{mod,window,protocol,assets,downloads}.rs`, `facturacion desktop` subcommand in `src/cli.rs`, default-args behavior, Windows failure dialog)
   - Window sizes, custom protocol with worker pool, SPA fallback, navigation lock, download handler, startup order, close handling.
7. **UI adaptation** (a copy of `ui/` in the Rust repo, for example `desktop-ui/`, plus a build script equivalent to `build-desktop.sh` with the `localhost:8090` guard)
   - NT card, strings, health names, tests under `tests/unit`.

## Key Learnings

1. The Rust `proceso run` is still a skeleton, so the job runner depends on the future orchestrator emitting events like Go's `procesoEvent` types.
2. The `/api/run` body uses capitalized keys (`Cargue`, `From`, `To`, `Limit`), and Go's JSON decoder matches keys case-insensitively, so Rust must accept both spellings.
3. Go stores date, SSC, subplan, authorization and total as columns on `cargue_factura` and filters in SQL, but Rust's `dataset_factura` has none of them.
4. The `shutdown.rs` flag in Rust is process-wide and one-way, so a UI cancel needs a separate per-job token.
5. The existing `ui/dist` already contains no absolute API URL, so it can be embedded as is.