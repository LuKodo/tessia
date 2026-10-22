//! Shared helpers of the `cuv_*` integration tests (included with
//! `#[path = "cuv_support.rs"] mod support;`).
#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use base64::Engine;
use serde_json::Value;
use tessia::context::AppContext;
use tessia::db::test_support::TestDb;
use tessia::domain::{Outcome, Phase};
use tessia::stores::proceso::{ProcesoState, SaveRow};
use tessia::testing::{MockResponse, MockServer};

pub const CARGUE: &str = "Convenio_21102026_21102026";

pub fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/cuv")
        .join(rel)
}

pub fn read(rel: &str) -> Vec<u8> {
    fs::read(fixture(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

pub fn jwt(expires_in: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default();
    let engine = &base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let payload = engine.encode(format!(r#"{{"exp":{}}}"#, now + expires_in));
    format!("{}.{payload}.sig", engine.encode(r#"{"alg":"none"}"#))
}

/// What the fake SISPRO received for an invoice.
#[derive(Debug, Clone)]
pub struct Received {
    pub invoice: String,
    pub rips: Vec<u8>,
    pub xml: Vec<u8>,
}

/// A local SISPRO: login, `CargarFevRips` answered per invoice (the
/// `numFactura` of the RIPS JSON) and `ConsultarCUV` answering ProcesoId 4242.
pub struct FakeSispro {
    pub server: MockServer,
    pub received: Arc<Mutex<Vec<Received>>>,
}

impl FakeSispro {
    /// `answers`: invoice -> (status, body); anything else gets a 404 without list.
    pub fn start(answers: HashMap<String, (u16, String)>) -> FakeSispro {
        Self::start_with(answers, true)
    }

    /// `login_ok = false` answers every login with a 401.
    pub fn start_with(answers: HashMap<String, (u16, String)>, login_ok: bool) -> FakeSispro {
        let received = Arc::new(Mutex::new(Vec::new()));
        let log = received.clone();
        let server = MockServer::start(move |req| match req.path.as_str() {
            "/api/Auth/LoginSISPRO" if login_ok => {
                MockResponse::json(200, &format!(r#"{{"token":"{}"}}"#, jwt(3600)))
            }
            "/api/Auth/LoginSISPRO" => MockResponse::json(401, r#"{"mensaje":"credenciales"}"#),
            "/api/ConsultasFevRips/ConsultarCUV" => {
                MockResponse::json(200, r#"{"ProcesoId":4242}"#)
            }
            "/api/PaquetesFevRips/CargarFevRips" => {
                let doc: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
                let invoice = doc["rips"]["numFactura"].as_str().unwrap_or("?").to_owned();
                let xml = base64::engine::general_purpose::STANDARD
                    .decode(doc["xmlFevFile"].as_str().unwrap_or(""))
                    .unwrap_or_default();
                let rips = rips_bytes(&req.body);
                if let Ok(mut log) = log.lock() {
                    log.push(Received {
                        invoice: invoice.clone(),
                        rips,
                        xml,
                    });
                }
                match answers.get(&invoice) {
                    Some((status, body)) => MockResponse::json(*status, body),
                    None => MockResponse::json(404, r#"{"ResultadosValidacion":[]}"#),
                }
            }
            _ => MockResponse::text(404, ""),
        });
        FakeSispro { server, received }
    }

    pub fn url(&self) -> &str {
        self.server.url()
    }

    pub fn count(&self, path: &str) -> usize {
        self.server
            .requests()
            .iter()
            .filter(|r| r.path == path)
            .count()
    }

    pub fn cargas(&self) -> Vec<Received> {
        self.received.lock().map(|l| l.clone()).unwrap_or_default()
    }

    pub fn sent_invoices(&self) -> Vec<String> {
        let mut invoices: Vec<String> = self.cargas().into_iter().map(|r| r.invoice).collect();
        invoices.sort();
        invoices
    }
}

/// The raw bytes of the `rips` member of a `{"rips": …, "xmlFevFile": …}` body.
fn rips_bytes(body: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(body);
    let start = text
        .find(r#"{"rips":"#)
        .map_or(0, |i| i + r#"{"rips":"#.len());
    let end = text.rfind(r#","xmlFevFile":""#).unwrap_or(text.len());
    text[start..end].as_bytes().to_vec()
}

pub const LOGIN: &str = "/api/Auth/LoginSISPRO";
pub const CARGAR: &str = "/api/PaquetesFevRips/CargarFevRips";
pub const CONSULTA: &str = "/api/ConsultasFevRips/ConsultarCUV";

/// A context whose `minsalud.*` point at `base_url` (empty = not configured).
pub fn context(db: &TestDb, base_url: &str, contrato: &str) -> AppContext {
    let ctx = AppContext::for_tests(db);
    let mut settings = ctx
        .settings
        .clone()
        .with("minsalud.numero_contrato", contrato);
    if !base_url.is_empty() {
        settings = settings
            .with("minsalud.base_url", base_url)
            .with("minsalud.tipo_doc", "CC")
            .with("minsalud.numero_doc", "123456")
            .with("minsalud.clave", "secret-pw")
            .with("minsalud.nit", "900123456");
    }
    ctx.with_settings(settings)
}

/// The RIPS JSON of an invoice with the given régimen.
pub fn rips_json(invoice: &str, regimen: &str) -> Vec<u8> {
    format!(
        r#"{{"numDocumentoIdObligado":"900123456","numFactura":"{invoice}","usuarios":[{{"tipoUsuario":"{regimen}","consecutivo":1}}]}}"#
    )
    .into_bytes()
}

pub fn publish(ctx: &AppContext, rel: &str, data: &[u8]) {
    ctx.artifacts()
        .publish_bytes(CARGUE, rel, data)
        .unwrap_or_else(|e| panic!("publish {rel}: {e:#}"));
}

pub fn fetch(ctx: &AppContext, rel: &str) -> Vec<u8> {
    ctx.artifacts()
        .fetch_bytes(CARGUE, rel)
        .unwrap_or_else(|e| panic!("fetch {rel}: {e:#}"))
}

pub fn published(ctx: &AppContext, rel: &str) -> bool {
    ctx.artifacts().is_published(CARGUE, rel).unwrap_or(false)
}

pub fn state(ctx: &AppContext, phase: Phase, invoice: &str) -> Option<ProcesoState> {
    ctx.proceso()
        .load(phase, Some(&[invoice.to_owned()]))
        .unwrap_or_default()
        .remove(invoice)
}

pub fn save_state(ctx: &AppContext, phase: Phase, invoice: &str, outcome: Outcome) {
    ctx.proceso()
        .save(&SaveRow {
            invoice: invoice.to_owned(),
            phase,
            outcome,
            intentos: 1,
            cargue_id: Some(CARGUE.to_owned()),
        })
        .expect("save state");
}

/// A successful SISPRO answer: a CUV for `invoice`.
pub fn success_body(invoice: &str) -> String {
    format!(
        r#"{{"ResultState":true,"ProcesoId":99,"NumFactura":"{invoice}","CodigoUnicoValidacion":"0abc0abc0abc","CodigoUnicoValidacionToShow":"0abc0abc0abc","FechaRadicacion":"2026-08-06T10:11:12.345","RutaArchivos":"C:\\rips\\{invoice}","ResultadosValidacion":[{{"Clase":"NOTIFICACION","Codigo":"RVC019","Descripcion":"ok","Observaciones":null,"PathFuente":"Rips","Fuente":"Rips"}}]}}"#
    )
}

pub fn answers(list: &[(&str, u16, String)]) -> HashMap<String, (u16, String)> {
    list.iter()
        .map(|(inv, status, body)| ((*inv).to_owned(), (*status, body.clone())))
        .collect()
}
