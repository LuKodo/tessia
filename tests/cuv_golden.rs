//! Golden tests of the CUV TXT handling, with the (synthetic) goldens of the Go
//! port (themselves produced by the Java code): `Response::parse` + `encode`
//! against Gson, `sweep` against `barrerTxtCuvMinisterioAlFinalizar` and
//! `sweep_local` against `Vista_ActualizarCUVMinisterioLocal`.

use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;

use serde_json::Value;
use tessia::phases::cuv::model::Response;
use tessia::phases::cuv::sweep::{sweep, sweep_local};

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/cuv")
        .join(rel)
}

fn read(rel: &str) -> Vec<u8> {
    fs::read(fixture(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn manifest(rel: &str, key: &str) -> Vec<Value> {
    let doc: Value = serde_json::from_slice(&read(rel)).expect("manifest");
    doc[key].as_array().expect("cases").clone()
}

fn text<'a>(case: &'a Value, key: &str) -> &'a str {
    case[key].as_str().unwrap_or_else(|| panic!("{key}"))
}

fn assert_text(got: &str, want: &[u8], name: &str) {
    assert_eq!(
        got,
        String::from_utf8_lossy(want),
        "{name}: bytes differ from the golden"
    );
}

#[test]
fn response_txt_matches_gson() {
    let cases = manifest("respuestas.json", "respuestas");
    assert_eq!(cases.len(), 5);
    for case in cases {
        let name = text(&case, "name");
        let response = Response::parse(text(&case, "body").as_bytes()).expect("parse");
        assert_text(&response.encode(), &read(text(&case, "txt")), name);
    }
}

#[test]
fn sweep_matches_the_java_final_sweep() {
    for case in manifest("respuestas.json", "respuestas") {
        let name = text(&case, "name");
        let asked = RefCell::new(Vec::new());
        let lookup = |cuv: &str| {
            asked.borrow_mut().push(cuv.to_owned());
            Some(4242)
        };
        let swept = sweep(&read(text(&case, "txt")), Some(&lookup)).expect("sweep");
        assert_text(
            &swept.out.expect("out"),
            &read(text(&case, "barrido")),
            name,
        );
        let want = case["cuv_rvg18"].as_str().unwrap_or("");
        assert_eq!(swept.cuv, want, "{name}");
        let asked = asked.into_inner();
        if want.is_empty() {
            assert!(asked.is_empty(), "{name}: no lookup without RVG18");
        } else {
            assert_eq!(asked, [want], "{name}");
        }
    }
}

#[test]
fn sweep_local_matches_the_java_screen() {
    let cases = manifest("local.json", "local");
    assert_eq!(cases.len(), 9);
    for case in cases {
        let name = text(&case, "name");
        let input = read(text(&case, "input"));
        let asked = RefCell::new(Vec::new());
        let lookup = |cuv: &str| {
            asked.borrow_mut().push(cuv.to_owned());
            Some(4242)
        };
        let result = sweep_local(&input, Some(&lookup));
        match text(&case, "resultado") {
            "no_json" => assert!(result.expect(name).out.is_none(), "{name}"),
            "error" => assert!(result.is_err(), "{name}: must be an error"),
            _ => {
                let swept = result.expect(name);
                assert_text(&swept.out.expect("out"), &read(text(&case, "output")), name);
                let want_cuv = case["cuv"].as_str();
                assert_eq!(swept.rvg18, want_cuv.is_some(), "{name}");
                assert_eq!(swept.cuv, want_cuv.unwrap_or(""), "{name}");
                assert_eq!(asked.borrow().len(), usize::from(swept.rvg18), "{name}");
            }
        }
    }
}
