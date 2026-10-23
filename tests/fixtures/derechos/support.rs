//! Shared helpers of the `derechos` integration tests (included with
//! `#[path = "fixtures/derechos/support.rs"] mod support;`). All data is
//! synthetic.
#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tessia::config::Settings;
use tessia::phases::derechos::LineInput;
use tessia::phases::derechos::service::InvoiceInput;
use tessia::testing::{MockResponse, MockServer, RecordedRequest};

/// A FHIR bundle with one patient, as the validator returns it.
pub fn bundle(modality: &str, zone: Option<&str>, city: &str, gender: &str, birth: &str) -> String {
    let zone_ext = zone.map_or(String::new(), |z| {
        format!(r#""extension":[{{"url":"mutualSER/hl7/patient/zone","valueString":"{z}"}}],"#)
    });
    format!(
        r#"{{"resourceType":"Bundle","entry":[{{"resource":{{"extension":[{{"url":"mutualSER/hl7/patient/healthModality","valueCoding":{{"display":"{modality}"}}}}],"gender":"{gender}","birthDate":"{birth}","address":[{{{zone_ext}"city":"{city}"}}]}}}}]}}"#
    )
}

/// The complete patient used by most tests (subsidiado, urban, Cartagena, female).
pub fn complete_bundle() -> String {
    bundle(
        "SUBSIDIADO",
        Some("Urbana"),
        "13001|CARTAGENA",
        "female",
        "1990-01-31",
    )
}

/// What a scripted validator sees of a request.
pub struct Query {
    pub doc_type: String,
    pub doc_number: String,
    pub bearer: String,
}

/// A Keycloak + FHIR mock. `/token` issues `t1`, `t2`, ... (valid for
/// `expires_in` seconds); `/validate` calls `answer`.
pub fn server(
    expires_in: u32,
    answer: impl Fn(&Query) -> MockResponse + Send + 'static,
) -> MockServer {
    let issued = AtomicUsize::new(0);
    MockServer::start(move |req| match req.path.as_str() {
        "/token" => {
            let n = issued.fetch_add(1, Ordering::SeqCst) + 1;
            MockResponse::json(
                200,
                &format!(r#"{{"access_token":"t{n}","expires_in":{expires_in}}}"#),
            )
        }
        "/validate" => answer(&query_of(req)),
        _ => MockResponse::text(404, ""),
    })
}

fn query_of(req: &RecordedRequest) -> Query {
    let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or_default();
    let param = |name: &str| {
        body["parameter"]
            .as_array()
            .and_then(|list| list.iter().find(|p| p["name"] == name))
            .and_then(|p| p["valueString"].as_str())
            .unwrap_or_default()
            .to_owned()
    };
    Query {
        doc_type: param("documentType"),
        doc_number: param("documentId"),
        bearer: req
            .header("authorization")
            .unwrap_or_default()
            .trim_start_matches("Bearer ")
            .to_owned(),
    }
}

pub fn token_requests(server: &MockServer) -> usize {
    server
        .requests()
        .iter()
        .filter(|r| r.path == "/token")
        .count()
}

pub fn validate_requests(server: &MockServer) -> Vec<RecordedRequest> {
    server
        .requests()
        .into_iter()
        .filter(|r| r.path == "/validate")
        .collect()
}

/// `derechos.*` settings pointing at `server`.
pub fn settings(server: &MockServer) -> Settings {
    Settings::default()
        .with("derechos.token_url", &format!("{}/token", server.url()))
        .with(
            "derechos.validate_url",
            &format!("{}/validate", server.url()),
        )
        .with("derechos.client_id", "client")
        .with("derechos.client_secret", "secret")
        .with("derechos.username", "user")
        .with("derechos.password", "pass")
}

/// An invoice with one line of the affiliate `doc_type doc_number`.
pub fn invoice(name: &str, doc_type: &str, doc_number: &str) -> InvoiceInput {
    InvoiceInput {
        invoice: name.into(),
        lines: vec![LineInput {
            doc_type: doc_type.into(),
            doc_number: doc_number.into(),
            ..LineInput::default()
        }],
    }
}

pub fn shared_counter() -> Arc<AtomicUsize> {
    Arc::new(AtomicUsize::new(0))
}
