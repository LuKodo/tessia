//! Read models A (`/api/cargues`, `/api/resumen`, `/api/seguimiento/consulta`,
//! `/api/facturas/{f}/seguimiento`) through `ApiState::handle` against a real
//! Postgres (`TestDb`). The fixture is a synthetic equivalent of the Go golden
//! fixture; the expected JSON below is the shape of
//! `cmd/tessia/testdata/golden/dashboard/*.json` (synthetic data only).

mod api_read_a_support;

use api_read_a_support::*;
use serde_json::{Value, json};

const KPI: [(&str, &str); 2] = [("kpi_desde", "2026-09-01"), ("kpi_hasta", "2026-09-30")];

fn consulta(f: &Fixture, extra: &[(&str, &str)]) -> (u16, Value) {
    let mut q: Vec<(&str, &str)> = KPI.to_vec();
    q.extend_from_slice(extra);
    get(&f.state, "/api/seguimiento/consulta", &q)
}

fn ok(result: (u16, Value)) -> Value {
    assert_eq!(result.0, 200, "{}", result.1);
    result.1
}

fn invoices(body: &Value) -> Vec<&str> {
    body["filas"]
        .as_array()
        .expect("filas")
        .iter()
        .map(|r| r["factura"].as_str().expect("factura"))
        .collect()
}

fn row<'a>(body: &'a Value, name: &str) -> &'a Value {
    body["filas"]
        .as_array()
        .expect("filas")
        .iter()
        .find(|r| r["factura"] == name)
        .unwrap_or_else(|| panic!("no row {name}"))
}

#[test]
fn cargues_match_the_golden_list() {
    let f = golden_fixture();
    let body = ok(get(&f.state, "/api/cargues", &[]));
    assert_eq!(
        body,
        json!([
            {"id":"CargueB","nombre":"CargueB.csv","fecha_inicio":"2026-09-26","fecha_fin":"2026-09-30",
             "facturas":2,"excluidas":0,"revision_manual":0,"modificado_en":"2026-09-28T08:00:00Z","estado":"PROCESANDO"},
            {"id":"CargueA","nombre":"CargueA.csv","fecha_inicio":"2026-09-01","fecha_fin":"2026-09-25",
             "facturas":7,"excluidas":1,"revision_manual":1,"modificado_en":"2026-09-20T08:00:00Z","estado":"PROCESANDO"},
            {"id":"CargueCSV","nombre":"Convenio_01092026_02092026.csv","fecha_inicio":"2026-09-01","fecha_fin":"2026-09-02",
             "facturas":0,"excluidas":0,"revision_manual":0,"modificado_en":"2026-09-05T08:00:00Z","estado":"SIN_PREPARAR"}
        ])
    );
}

#[test]
fn an_empty_database_answers_empty_arrays() {
    let f = empty_fixture();
    assert_eq!(ok(get(&f.state, "/api/cargues", &[])), json!([]));
    let body = ok(get(&f.state, "/api/seguimiento/consulta", &[]));
    assert_eq!(body["filas"], json!([]));
    assert_eq!(body["total"], 0);
    assert_eq!(body["facets"], json!({"estados": [], "alertas": {}}));
    assert_eq!(body["next_cursor"], Value::Null);
    let summary = ok(get(&f.state, "/api/resumen", &[]));
    assert_eq!(summary["cargues"], json!([]));
}

#[test]
fn the_cargue_status_rolls_up_the_prepared_invoices() {
    let db = tessia::db::test_support::TestDb::new();
    let state = state_for(&db);
    let load =
        |file: &str,
         hash,
         b: fn(
            tessia::domain::dataset::test_support::DatasetBuilder,
        ) -> tessia::domain::dataset::test_support::DatasetBuilder,
         at: &str| { store_load(&db, file, hash, b, "2026-09-01", "2026-09-02", at) };
    load(
        "Done.csv",
        'a',
        |b| b.invoice(tessia::domain::dataset::test_support::InvoiceBuilder::new("DON1").line("1")),
        "2026-09-01T00:00:00Z",
    );
    load(
        "Mixed.csv",
        'b',
        |b| {
            b.invoice(tessia::domain::dataset::test_support::InvoiceBuilder::new("MIX1").line("1"))
                .invoice(
                    tessia::domain::dataset::test_support::InvoiceBuilder::new("MIX2").line("1"),
                )
        },
        "2026-09-02T00:00:00Z",
    );
    load(
        "Busy.csv",
        'c',
        |b| {
            b.invoice(tessia::domain::dataset::test_support::InvoiceBuilder::new("BUS1").line("1"))
                .invoice(
                    tessia::domain::dataset::test_support::InvoiceBuilder::new("BUS2").line("1"),
                )
        },
        "2026-09-03T00:00:00Z",
    );
    load("NoInv.csv", 'd', |b| b, "2026-09-04T00:00:00Z");
    load(
        "OnlyExcl.csv",
        'e',
        |b| b.excluded("EXC1", "X"),
        "2026-09-05T00:00:00Z",
    );
    radicacion(&db, "DON1", 4, "CONFIRMED", "", "2026-09-10 10:00:00");
    radicacion(&db, "MIX1", 4, "CONFIRMED", "", "2026-09-10 10:00:00");
    radicacion(
        &db,
        "MIX2",
        3,
        "ADJUSTMENT_NOTE",
        "x",
        "2026-09-10 10:00:00",
    );
    radicacion(&db, "BUS1", 4, "CONFIRMED", "", "2026-09-10 10:00:00");
    let body = ok(get(&state, "/api/cargues", &[]));
    let status: Vec<(&str, &str)> = body
        .as_array()
        .expect("list")
        .iter()
        .map(|c| {
            (
                c["id"].as_str().expect("id"),
                c["estado"].as_str().expect("estado"),
            )
        })
        .collect();
    assert_eq!(
        status,
        [
            ("OnlyExcl", "CON_INCIDENCIAS"),
            ("NoInv", "SIN_FACTURAS"),
            ("Busy", "PROCESANDO"),
            ("Mixed", "CON_INCIDENCIAS"),
            ("Done", "COMPLETADO"),
        ]
    );
}

#[test]
fn only_the_current_cargue_of_a_load_is_listed_and_the_list_is_cut_at_twenty() {
    let db = tessia::db::test_support::TestDb::new();
    let state = state_for(&db);
    let dsb = tessia::domain::dataset::test_support::InvoiceBuilder::new("OLD1").line("1");
    store_load(
        &db,
        "Load.csv",
        'a',
        |b| b.invoice(dsb),
        "2026-09-01",
        "2026-09-02",
        "2026-09-01T00:00:00Z",
    );
    // A newer upload of the same load that was never prepared must not shadow it.
    let info = tessia::domain::cargue::load_context("Load.csv");
    tessia::stores::cargues::CarguesStore::new(db.pool())
        .ingest(&tessia::stores::cargues::NewCargue::from_load(
            &info,
            &"f".repeat(64),
        ))
        .expect("ingest");
    let body = ok(get(&state, "/api/cargues", &[]));
    let list = body.as_array().expect("list");
    assert_eq!(list.len(), 1);
    assert_eq!(
        (list[0]["facturas"].clone(), list[0]["estado"].clone()),
        (json!(1), json!("PROCESANDO"))
    );
    for i in 0..22 {
        let name = format!("Many{i:02}.csv");
        let hash = char::from_digit(i % 16, 16).expect("hex");
        // distinct hashes are only needed per name
        store_load(
            &db,
            &name,
            hash,
            |b| b,
            "2026-09-01",
            "2026-09-02",
            &format!("2026-10-{:02}T00:00:00Z", i + 1),
        );
    }
    let body = ok(get(&state, "/api/cargues", &[]));
    let list = body.as_array().expect("list");
    assert_eq!(list.len(), 20);
    assert_eq!(list[0]["id"], "Many21");
}

#[test]
fn resumen_matches_the_golden_summary() {
    let f = golden_fixture();
    let body = ok(get(
        &f.state,
        "/api/resumen",
        &[("desde", "2026-09-01"), ("hasta", "2026-09-30")],
    ));
    assert_eq!(
        body,
        json!({
            "desde":"2026-09-01","hasta":"2026-09-30",
            "kpis":{"total":9,"no_radicado":2,"radicadas":2,"rechazadas":4,"radicacion_parcial":1,"nota_ajuste":1,
                    "listas_para_radicar":0,"nota_credito":2,"en_proceso":0,"sin_autorizacion":0},
            "cargues":[
                {"cargue":"CargueB","nombre":"CargueB.csv","fecha_inicio":"2026-09-26","total":2,"gestionadas":2,
                 "excluidas":0,"revision_manual":0,"radicadas":1,"valor_cargado":"220","valor_radicado":"110"},
                {"cargue":"CargueA","nombre":"CargueA.csv","fecha_inicio":"2026-09-01","total":9,"gestionadas":7,
                 "excluidas":1,"revision_manual":1,"radicadas":1,"valor_cargado":"770","valor_radicado":"110"}
            ]
        })
    );
}

#[test]
fn resumen_kpis_follow_the_range_but_the_chart_ignores_it() {
    let f = golden_fixture();
    let body = ok(get(
        &f.state,
        "/api/resumen",
        &[("desde", "2026-10-01"), ("hasta", "2026-10-31")],
    ));
    assert_eq!(body["kpis"]["total"], 0);
    assert_eq!(body["cargues"].as_array().expect("cargues").len(), 2);
    let (status, err) = get(
        &f.state,
        "/api/resumen",
        &[("desde", "2026-10-02"), ("hasta", "2026-10-01")],
    );
    assert_eq!(
        (status, err),
        (400, json!({"error": "hasta es anterior a desde"}))
    );
    let (status, err) = get(
        &f.state,
        "/api/resumen",
        &[("desde", "2025-01-01"), ("hasta", "2026-10-01")],
    );
    assert_eq!(
        (status, err),
        (400, json!({"error": "el rango no puede superar 366 días"}))
    );
    let (status, err) = get(&f.state, "/api/resumen", &[("hasta", "x")]);
    assert_eq!(
        (status, err),
        (400, json!({"error": "--hasta must be YYYY-MM-DD"}))
    );
}

#[test]
fn consulta_default_page_matches_the_golden_rows_and_aggregates() {
    let f = golden_fixture();
    let body = ok(consulta(&f, &[]));
    assert_eq!(body["total"], 9);
    assert_eq!(
        invoices(&body),
        [
            "FVS1", "FVS2", "FVS3", "FVS4", "FVS5", "FVS6", "FVS7", "FVS20", "FVS21"
        ]
    );
    assert_eq!(body["next_cursor"], Value::Null);
    assert_eq!(
        body["kpis"],
        json!({"total":9,"no_radicado":2,"radicadas":2,"rechazadas":4,"radicacion_parcial":1,"nota_ajuste":1,
               "listas_para_radicar":0,"nota_credito":2,"en_proceso":0,"sin_autorizacion":0})
    );
    assert_eq!(
        body["facets"],
        json!({
            "estados":["EN_PROCESO","EXCLUIDA","NOTA_AJUSTE","RECHAZADO","REVISION_MANUAL","SIN INICIAR","TERMINADO","VENCIDO"],
            "alertas":{"falta_crc":7,"falta_cuv":5,"falta_json":6,"falta_zip":3,"nota_ajuste":1,"nota_credito":2,
                       "pendiente_radicar":3,"vencida_22_dias":1}
        })
    );
    assert_eq!(
        *row(&body, "FVS1"),
        json!({
            "factura":"FVS1","resultado":"preparada","cargue":"CargueA","cargue_nombre":"CargueA.csv",
            "artefactos":["ok","ok","ok","ok"],"radicacion":["ok","ok","ok","ok"],
            "estado":"TERMINADO","estado_detalle":"Radicada completa","fecha_factura":"2026-09-03","ssc":"SSC1",
            "subplan":"EVENTO NO PBS","valor_convenio":"110","autorizacion":"AUT-FVS1","id_mipres":"111, 222",
            "actualizado_en":"2026-09-10T10:00:00Z","pipeline":"radicada",
            "fecha_inicio_cargue":"2026-09-01","fecha_fin_cargue":"2026-09-25",
            "portal_consultado":true,"portal_estado":"Aprobado","portal_advertencias":1,"portal_final":true
        })
    );
    assert_eq!(
        *row(&body, "FVS4"),
        json!({
            "factura":"FVS4","resultado":"preparada","cargue":"CargueA","cargue_nombre":"CargueA.csv",
            "artefactos":["pendiente","pendiente","pendiente","pendiente"],"radicacion":["ok","ok","error","pendiente"],
            "estado":"NOTA_AJUSTE","estado_detalle":"requiere nota ajuste","fecha_factura":"2026-09-03","ssc":"SSC4",
            "subplan":"EVENTO POS","valor_convenio":"110","autorizacion":"AUT-FVS4","id_mipres":"No aplica",
            "actualizado_en":"2026-09-13T08:10:00Z","requiere_nota_ajuste":true,"pipeline":"rechazada",
            "alertas":["falta_json","falta_crc","nota_ajuste"],
            "fecha_inicio_cargue":"2026-09-01","fecha_fin_cargue":"2026-09-25","portal_consultado":false
        })
    );
    assert_eq!(
        *row(&body, "FVS5"),
        json!({
            "factura":"FVS5","resultado":"preparada","cargue":"CargueA","cargue_nombre":"CargueA.csv",
            "artefactos":["ok","pendiente","pendiente","pendiente"],"radicacion":["error","pendiente","pendiente","pendiente"],
            "estado":"VENCIDO","estado_detalle":"RFFE022: superó los 22 días hábiles","fecha_factura":"2026-09-03",
            "ssc":"SSC5","subplan":"EVENTO POS","valor_convenio":"110","autorizacion":"AUT-FVS5","id_mipres":"No aplica",
            "actualizado_en":"2026-09-14T08:12:00Z","requiere_nota_credito":true,"vencida_rffe022":true,
            "pipeline":"rechazada","alertas":["falta_json","falta_cuv","falta_crc","nota_credito","vencida_22_dias"],
            "fecha_inicio_cargue":"2026-09-01","fecha_fin_cargue":"2026-09-25","portal_consultado":false
        })
    );
    assert_eq!(
        *row(&body, "FVS7"),
        json!({
            "factura":"FVS7","resultado":"preparada","cargue":"CargueA","cargue_nombre":"CargueA.csv",
            "artefactos":["pendiente","pendiente","pendiente","pendiente"],
            "radicacion":["pendiente","pendiente","pendiente","pendiente"],
            "fecha_factura":"2026-09-03","subplan":"EVENTO POS","valor_convenio":"110","autorizacion":"AUT-FVS7",
            "id_mipres":"No aplica","pipeline":"no_radicado",
            "alertas":["falta_zip","falta_json","falta_cuv","falta_crc","pendiente_radicar"],
            "fecha_inicio_cargue":"2026-09-01","fecha_fin_cargue":"2026-09-25","portal_consultado":false
        })
    );
    // The list row cuts the detail; a filed invoice shows its real artifacts.
    assert_eq!(
        row(&body, "FVS3")["estado_detalle"],
        "Bloqueada por tarifa: un precio del XML supera la nota técnica. Requiere nota crédito y re…"
    );
    assert_eq!(
        row(&body, "FVS20")["artefactos"],
        json!(["pendiente", "pendiente", "pendiente", "ok"])
    );
    assert_eq!(row(&body, "FVS3")["portal_errores"], 2);
}

#[test]
fn consulta_aggregates_alone_for_excluded_rows_match_the_golden() {
    let f = golden_fixture();
    let body = ok(consulta(&f, &[("limit", "0"), ("pipeline", "no_aplica")]));
    assert_eq!(body["total"], 2);
    assert_eq!(body["filas"], json!([]));
    assert_eq!(body["next_cursor"], Value::Null);
    assert_eq!(
        body["kpis"],
        json!({"total":0,"no_radicado":0,"radicadas":0,"rechazadas":0,"radicacion_parcial":0,"nota_ajuste":0,
               "listas_para_radicar":0,"nota_credito":0,"en_proceso":0,"sin_autorizacion":0})
    );
    assert_eq!(body["facets"]["alertas"]["falta_crc"], 7);
    // The excluded and manual-review rows themselves.
    let rows = ok(consulta(&f, &[("pipeline", "no_aplica")]));
    assert_eq!(invoices(&rows), ["FVS8", "FVS9"]);
    assert_eq!(
        row(&rows, "FVS8")["estado_detalle"],
        "MONTO_MAXIMO_SUPERADO"
    );
    assert_eq!(row(&rows, "FVS8")["resultado"], "excluida");
    assert_eq!(row(&rows, "FVS9")["estado_detalle"], "SIN_SSC: sin ssc");
    assert_eq!(row(&rows, "FVS9")["resultado"], "revision_manual");
    assert!(row(&rows, "FVS9").get("alertas").is_none());
}

#[test]
fn consulta_alert_filter_with_the_update_sort_walks_pages_by_cursor() {
    let f = golden_fixture();
    let q = [
        ("alerta", "falta_json,nota_ajuste"),
        ("sort", "actualizado_en"),
        ("dir", "asc"),
        ("limit", "3"),
    ];
    let first = ok(consulta(&f, &q));
    // Rows with no native state sort first (empty key), then by update time.
    assert_eq!(invoices(&first), ["FVS7", "FVS21", "FVS2"]);
    assert_eq!(first["total"], 6);
    assert_eq!(first["kpis"]["total"], 6);
    assert_eq!(first["kpis"]["rechazadas"], 3);
    let cursor = first["next_cursor"].as_str().expect("cursor").to_owned();
    let mut next = q.to_vec();
    next.push(("cursor", &cursor));
    let second = ok(consulta(&f, &next));
    assert_eq!(invoices(&second), ["FVS3", "FVS4", "FVS5"]);
    assert_eq!(second["next_cursor"], Value::Null);
    // The golden cursor is base64url(JSON{k,c,f,s,d}).
    let raw = base64_url_decode(&cursor);
    let parsed: Value = serde_json::from_slice(&raw).expect("cursor json");
    assert_eq!(parsed["s"], "actualizado_en");
    assert_eq!(parsed["c"], "CargueA");
    assert_eq!(parsed["f"], "FVS2");
    assert!(parsed.get("d").is_none());
}

fn base64_url_decode(text: &str) -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(text)
        .expect("base64url")
}

#[test]
fn consulta_pages_cover_every_row_once_in_every_order() {
    let f = golden_fixture();
    type Order<'a> = (&'a [(&'a str, &'a str)], [&'a str; 9]);
    let orders: [Order; 4] = [
        (
            &[],
            [
                "FVS1", "FVS2", "FVS3", "FVS4", "FVS5", "FVS6", "FVS7", "FVS20", "FVS21",
            ],
        ),
        (
            &[("sort", "factura"), ("dir", "asc")],
            [
                "FVS1", "FVS2", "FVS20", "FVS21", "FVS3", "FVS4", "FVS5", "FVS6", "FVS7",
            ],
        ),
        (
            &[("sort", "factura")],
            [
                "FVS7", "FVS6", "FVS5", "FVS4", "FVS3", "FVS21", "FVS20", "FVS2", "FVS1",
            ],
        ),
        // All dates are equal: ties break by (cargue, factura) ascending in both directions.
        (
            &[("sort", "fecha_factura")],
            [
                "FVS1", "FVS2", "FVS3", "FVS4", "FVS5", "FVS6", "FVS7", "FVS20", "FVS21",
            ],
        ),
    ];
    for (order, want) in orders {
        let mut seen: Vec<String> = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut q: Vec<(&str, &str)> = order.to_vec();
            q.push(("limit", "4"));
            q.push(("agregados", "no"));
            if let Some(c) = &cursor {
                q.push(("cursor", c));
            }
            let body = ok(consulta(&f, &q));
            assert!(
                body.get("total").is_none()
                    && body.get("kpis").is_none()
                    && body.get("facets").is_none()
            );
            seen.extend(invoices(&body).into_iter().map(str::to_owned));
            match body["next_cursor"].as_str() {
                Some(c) => cursor = Some(c.to_owned()),
                None => break,
            }
        }
        assert_eq!(seen, want, "order {order:?}");
    }
}

#[test]
fn consulta_text_estado_pipeline_and_date_filters() {
    let f = golden_fixture();
    let names = |extra: &[(&str, &str)]| {
        let body = ok(consulta(&f, extra));
        invoices(&body)
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&[("factura", " fvs2 ")]), ["FVS2", "FVS20", "FVS21"]);
    assert_eq!(names(&[("cargue", "cargueb")]), ["FVS20", "FVS21"]);
    assert_eq!(names(&[("cargue", "cargueb.csv carg")]), ["FVS20", "FVS21"]);
    assert_eq!(names(&[("ssc", "ssc6")]), ["FVS6"]);
    assert_eq!(names(&[("autorizacion", "AUT-FVS5")]), ["FVS5"]);
    assert_eq!(names(&[("mipres", "111")]), ["FVS1"]);
    assert_eq!(
        names(&[("mipres", "no aplica"), ("limit", "2")]),
        ["FVS2", "FVS3"]
    );
    assert_eq!(names(&[("subplan", "no pbs")]), ["FVS1"]);
    assert_eq!(names(&[("estado", "TERMINADO")]), ["FVS1", "FVS20"]);
    assert_eq!(names(&[("estado", "SIN INICIAR")]), ["FVS7", "FVS21"]);
    assert_eq!(
        names(&[("estado", "terminado")]),
        Vec::<String>::new(),
        "estado is exact"
    );
    assert_eq!(names(&[("estado", "EXCLUIDA")]), ["FVS8"]);
    assert_eq!(names(&[("estado", "REVISION_MANUAL")]), ["FVS9"]);
    assert_eq!(names(&[("pipeline", "radicada")]), ["FVS1", "FVS20"]);
    assert_eq!(names(&[("alerta", "vencida_22_dias")]), ["FVS5"]);
    assert_eq!(
        names(&[("alerta", "nota_credito"), ("pipeline", "rechazada")]),
        ["FVS5", "FVS6"]
    );
    // The date filter drops rows without a date: excluded rows with no pipeline asked never show.
    assert_eq!(
        names(&[("desde", "2026-09-03"), ("hasta", "2026-09-03")]).len(),
        9
    );
    assert_eq!(names(&[("desde", "2026-09-04")]), Vec::<String>::new());
    assert_eq!(names(&[("hasta", "2026-09-02")]), Vec::<String>::new());
    assert_eq!(
        names(&[("pipeline", "no_aplica"), ("hasta", "2026-09-30")]),
        Vec::<String>::new()
    );
}

#[test]
fn consulta_kpis_follow_the_month_bounds_unless_a_cargue_filter_is_set() {
    let f = golden_fixture();
    let get_kpis = |q: &[(&str, &str)]| {
        let body = ok(get(&f.state, "/api/seguimiento/consulta", q));
        body["kpis"]["total"].as_u64().expect("total")
    };
    assert_eq!(
        get_kpis(&[("kpi_desde", "2026-10-01"), ("kpi_hasta", "2026-10-31")]),
        0
    );
    assert_eq!(
        get_kpis(&[
            ("kpi_desde", "2026-10-01"),
            ("kpi_hasta", "2026-10-31"),
            ("cargue", "cargueb")
        ]),
        2
    );
    // Only an upper bound: a row without a date would pass, rows dated after it fail.
    assert_eq!(get_kpis(&[("kpi_hasta", "2026-09-02")]), 0);
    assert_eq!(get_kpis(&[("kpi_hasta", "2026-09-03")]), 9);
    assert_eq!(get_kpis(&[("kpi_desde", "2026-09-03")]), 9);
    assert_eq!(get_kpis(&[("kpi_desde", "2026-09-04")]), 0);
}

#[test]
fn consulta_rejects_bad_parameters_with_the_go_messages() {
    let f = golden_fixture();
    let bad = |q: &[(&str, &str)]| {
        let (status, body) = get(&f.state, "/api/seguimiento/consulta", q);
        assert_eq!(status, 400);
        body["error"].as_str().expect("error").to_owned()
    };
    assert_eq!(bad(&[("pipeline", "x")]), "pipeline desconocido \"x\"");
    assert_eq!(bad(&[("alerta", "zzz")]), "alerta desconocida \"zzz\"");
    assert_eq!(bad(&[("dir", "up")]), "dir debe ser asc o desc");
    assert_eq!(
        bad(&[("limit", "1001")]),
        "limit debe ser un entero entre 0 y 1000"
    );
    assert_eq!(
        bad(&[("limit", "0"), ("agregados", "no")]),
        "limit=0 solo pide los agregados: no puede combinarse con agregados=no"
    );
    assert_eq!(bad(&[("cursor", "no-es-un-cursor")]), "cursor invalido");
    let (_, page) = (0, ok(consulta(&f, &[("sort", "factura"), ("limit", "2")])));
    let cursor = page["next_cursor"].as_str().expect("cursor");
    assert_eq!(
        bad(&[("sort", "factura"), ("dir", "asc"), ("cursor", cursor)]),
        "cursor no corresponde al orden pedido"
    );
}

#[test]
fn the_detail_matches_the_golden_invoice() {
    let f = golden_fixture();
    let expected = json!({
        "factura":"FVS1","resultado":"preparada","cargue":"CargueA","cargue_nombre":"CargueA.csv",
        "artefactos":["ok","ok","ok","ok"],"radicacion":["ok","ok","ok","ok"],
        "estado":"TERMINADO","estado_detalle":"Radicada completa","fecha_factura":"2026-09-03","ssc":"SSC1",
        "subplan":"EVENTO NO PBS","valor_convenio":"110","autorizacion":"AUT-FVS1","id_mipres":"111, 222",
        "actualizado_en":"2026-09-10T10:00:00Z","pipeline":"radicada",
        "fecha_inicio_cargue":"2026-09-01","fecha_fin_cargue":"2026-09-25",
        "portal_consultado":true,"portal_estado":"Aprobado","portal_advertencias":1,"portal_final":true,
        "lineas_convenio":[
            {"ssc":"SSC1","cum":"CUM1","valor_unitario":"40.25","valor_total":"100.50","autorizacion":"AUT-FVS1","id_mipres":"M1"},
            {"ssc":"SSC1","cum":"CUM2","valor_unitario":"7","valor_total":"9.5","autorizacion":"","id_mipres":""}
        ]
    });
    let body = ok(get(
        &f.state,
        "/api/facturas/FVS1/seguimiento",
        &[("cargue", "CargueA")],
    ));
    assert_eq!(body, expected);
    // Key order is the Go struct order.
    let keys: Vec<&String> = body.as_object().expect("object").keys().collect();
    assert_eq!(keys.first().map(|k| k.as_str()), Some("factura"));
    assert_eq!(keys.last().map(|k| k.as_str()), Some("lineas_convenio"));
    // The path is normalized and the cargue is optional (newest holding it answers).
    assert_eq!(
        ok(get(&f.state, "/api/facturas/fvs-1/seguimiento", &[])),
        expected
    );
}

#[test]
fn the_detail_keeps_the_whole_estado_text() {
    let f = golden_fixture();
    let body = ok(get(&f.state, "/api/facturas/FVS3/seguimiento", &[]));
    assert_eq!(
        body["estado_detalle"],
        "Bloqueada por tarifa: un precio del XML supera la nota técnica. Requiere nota crédito y refacturación."
    );
    let excluded = ok(get(&f.state, "/api/facturas/FVS8/seguimiento", &[]));
    assert_eq!(excluded["resultado"], "excluida");
    assert_eq!(excluded["estado_detalle"], "MONTO_MAXIMO_SUPERADO");
    assert_eq!(excluded["pipeline"], "no_aplica");
    assert!(excluded.get("lineas_convenio").is_none());
}

#[test]
fn the_detail_reports_the_go_errors() {
    let f = golden_fixture();
    let err = |path: &str, q: &[(&str, &str)]| {
        let (status, body) = get(&f.state, path, q);
        (status, body["error"].as_str().expect("error").to_owned())
    };
    assert_eq!(
        err("/api/facturas/XX1/seguimiento", &[]),
        (400, "factura inválida".into())
    );
    assert_eq!(
        err("/api/facturas/FVS1/seguimiento", &[("cargue", "a/b")]),
        (400, "cargue inválido".into())
    );
    assert_eq!(
        err("/api/facturas/FVS1/seguimiento", &[("cargue", "..")]),
        (400, "cargue inválido".into())
    );
    assert_eq!(
        err("/api/facturas/FVS99/seguimiento", &[]),
        (404, "dashboard: invoice not found: \"FVS99\"".into())
    );
    assert_eq!(
        err("/api/facturas/FVS1/seguimiento", &[("cargue", "Nope")]),
        (404, "dashboard: cargue not found: \"Nope\"".into())
    );
    // The invoice exists, but in another cargue.
    assert_eq!(
        err("/api/facturas/FVS1/seguimiento", &[("cargue", "CargueB")]),
        (404, "dashboard: invoice not found: \"FVS1\"".into())
    );
    // An unprepared cargue has no dataset: Go reads it as a missing cargue.
    assert_eq!(
        err("/api/facturas/FVS1/seguimiento", &[("cargue", "CargueCSV")]),
        (404, "dashboard: cargue not found: \"CargueCSV\"".into())
    );
}

#[test]
fn the_stored_xml_total_is_shown_without_trailing_zeros() {
    let f = golden_fixture();
    exec(
        &f.db,
        "INSERT INTO factura_valor_xml (num_factura, xml_valor) VALUES ('FVS1', 110.50), ('FVS2', NULL)",
    );
    let body = ok(consulta(&f, &[]));
    assert_eq!(row(&body, "FVS1")["valor_xml"], "110.5");
    assert!(row(&body, "FVS2").get("valor_xml").is_none());
    let detail = ok(get(&f.state, "/api/facturas/FVS1/seguimiento", &[]));
    assert_eq!(detail["valor_xml"], "110.5");
}

#[test]
fn personal_data_never_reaches_the_responses() {
    let f = golden_fixture();
    exec(
        &f.db,
        "INSERT INTO derechos_afiliado (tipo_documento, numero_documento, tipo_usuario, zona, municipio_dane, sexo, fecha_nacimiento, validado_at) VALUES ('CC', '1000000001', 'C', 'U', '05001', 'F', '2000-01-01', now())",
    );
    for (path, q) in [
        ("/api/cargues", vec![]),
        ("/api/resumen", vec![]),
        ("/api/seguimiento/consulta", vec![]),
        ("/api/facturas/FVS1/seguimiento", vec![]),
    ] {
        let body = raw(&f.state, path, &q);
        for secret in [
            "1000000001",
            "PACIENTE DE PRUEBA",
            "2000-01-01",
            "derechos_afiliado",
        ] {
            assert!(!body.contains(secret), "{path} leaks {secret}");
        }
    }
}

#[test]
fn a_store_failure_is_a_generic_message() {
    let f = golden_fixture();
    exec(
        &f.db,
        "ALTER TABLE radicacion_estado RENAME TO radicacion_estado_x",
    );
    let (status, body) = get(&f.state, "/api/seguimiento/consulta", &[]);
    assert_eq!(status, 500);
    assert_eq!(
        body,
        json!({"error": "no se pudo consultar el seguimiento"})
    );
    let (_, cargues) = get(&f.state, "/api/cargues", &[]);
    assert_eq!(cargues, json!({"error": "no se pudieron leer los cargues"}));
    let (_, resumen) = get(&f.state, "/api/resumen", &[]);
    assert_eq!(resumen, json!({"error": "no se pudo calcular el resumen"}));
    let (_, one) = get(&f.state, "/api/facturas/FVS1/seguimiento", &[]);
    assert_eq!(
        one,
        json!({"error": "no se pudo leer el seguimiento de la factura"})
    );
}
