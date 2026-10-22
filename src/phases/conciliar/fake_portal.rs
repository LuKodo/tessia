//! A fake of the Medicar formula pages (login, formula list and formula
//! detail) with synthetic HTML modelled on what the client parses. No real
//! patient data. Used by this phase's unit and integration tests
//! (`test-support` feature).

use std::collections::HashMap;

use crate::testing::{MockResponse, MockServer, RecordedRequest};

/// Portal path the fake serves (`medicar.base_url` = server URL + this).
pub const BASE_PATH: &str = "/pharmaser/mutualser";
/// Credentials the fake accepts.
pub const IDENTIFIER: &str = "ID-SINT";
pub const USERNAME: &str = "usuario-sint";
pub const PASSWORD: &str = "clave-sint";
const SESSION: &str = "sess-sint";
const CSRF: &str = "tok&sint";

/// One formula the fake knows.
#[derive(Debug, Clone)]
pub struct FakeFormula {
    /// Patient document (digits).
    pub document: String,
    pub ssc: String,
    /// Invoice as shown in the list, e.g. `FVS-9000001`.
    pub invoice: String,
    /// `yyyy-MM-dd HH:mm:ss`.
    pub date: String,
    /// Detail rows: molecule, article, dispatch state, dispatch date, quantity.
    pub rows: Vec<[String; 5]>,
}

impl FakeFormula {
    /// The formula the tests share: document 900001, SSC 7770001.
    pub fn sample() -> Self {
        let row = |a: &str, b: &str, c: &str, d: &str, e: &str| [a, b, c, d, e].map(str::to_owned);
        Self {
            document: "900001".into(),
            ssc: "7770001".into(),
            invoice: "FVS-9000001".into(),
            date: "2026-08-15 10:20:30".into(),
            rows: vec![
                row(
                    "ACETAMINOFEN",
                    "ACETA 500 SINT",
                    "DESPACHADO",
                    "2026-08-16 08:00:00",
                    "10",
                ),
                row(
                    "ACETAMINOFEN/CODEINA",
                    "WINA SINT",
                    "DESPACHADO",
                    "2026-08-16 09:00:00",
                    "3,0",
                ),
            ],
        }
    }
}

/// The portal: knows its formulas (count the requests it served with
/// `MockServer::requests`).
#[derive(Default)]
pub struct FakePortal {
    formulas: Vec<FakeFormula>,
}

fn md5_hex(s: &str) -> String {
    use md5::{Digest, Md5};
    hex::encode(Md5::digest(s.as_bytes()))
}

fn form(req: &RecordedRequest) -> HashMap<String, String> {
    url::form_urlencoded::parse(&req.body)
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

impl FakePortal {
    pub fn new(formulas: Vec<FakeFormula>) -> Self {
        Self { formulas }
    }

    /// Starts a server answering with [`FakePortal::respond`].
    pub fn start(self) -> MockServer {
        MockServer::start(move |req| self.respond(req))
    }

    pub fn respond(&self, req: &RecordedRequest) -> MockResponse {
        let path = req.path.split('?').next().unwrap_or("");
        let has_session = req
            .header("cookie")
            .is_some_and(|c| c.contains(&format!("PHPSESSID={SESSION}")));
        match (
            req.method.as_str(),
            path.strip_prefix(BASE_PATH).unwrap_or("?"),
        ) {
            ("GET", "/") => MockResponse::text(
                200,
                &format!(
                    r#"<form><input type="hidden" value="{}" name="csrf_token"></form>"#,
                    CSRF.replace('&', "&amp;")
                ),
            )
            .with_header("Set-Cookie", &format!("PHPSESSID={SESSION}; Path=/")),
            _ if !has_session => MockResponse::text(403, "no session"),
            ("POST", "/procesar.php") => self.login(req),
            ("POST", "/modalAjax/Control.php") => self.list(req),
            ("POST", "/modalAjax/VerFormula.php") => self.detail(req),
            _ => MockResponse::text(404, "not found"),
        }
    }

    fn login(&self, req: &RecordedRequest) -> MockResponse {
        let f = form(req);
        let get = |k: &str| f.get(k).map(String::as_str).unwrap_or_default();
        if get("csrf_token") != CSRF
            || get("f_id") != IDENTIFIER
            || get("f_log") != md5_hex(USERNAME)
            || get("f_pass") != md5_hex(PASSWORD)
        {
            return MockResponse::text(200, r#"{"error":"login"}"#);
        }
        MockResponse::text(200, r#"{"status":1}"#)
    }

    fn list(&self, req: &RecordedRequest) -> MockResponse {
        let f = form(req);
        let mut out = String::from("<table><tr><th>SSC</th><th>Factura</th><th>Ver</th></tr>");
        for fm in &self.formulas {
            let asked = f.get("f_opciones").is_some_and(|o| o == "3")
                && f.get("f_numero").is_some_and(|n| *n == fm.document);
            if asked {
                out.push_str(&format!(
                    "<tr><td>{}</td><td>{}</td><td><a onclick=\"AjaxModalVerFormula('{}')\">ver</a></td></tr>",
                    fm.ssc, fm.invoice, fm.ssc
                ));
            }
        }
        out.push_str("</table>");
        MockResponse::text(200, &out)
    }

    fn detail(&self, req: &RecordedRequest) -> MockResponse {
        let f = form(req);
        let id = f.get("IdFormula").map(String::as_str).unwrap_or_default();
        let Some(fm) = self.formulas.iter().find(|fm| fm.ssc == id) else {
            return MockResponse::text(200, "<div>Sin datos</div>");
        };
        let mut out = format!(
            "<div><b>Formula SSC No:</b> {}<br>Fecha:&nbsp;{}</div><table>",
            fm.ssc, fm.date
        );
        out.push_str("<tr><th>#</th><th>Molecula</th><th>Articulo</th><th>Estado</th><th>Fecha despacho</th><th>Cantidad</th></tr>");
        for (i, r) in fm.rows.iter().enumerate() {
            out.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                i + 1,
                r[0],
                r[1],
                r[2],
                r[3],
                r[4]
            ));
        }
        out.push_str("</table>");
        MockResponse::text(200, &out)
    }
}
