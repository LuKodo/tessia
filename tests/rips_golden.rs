//! `rips::encode` against the Java goldens of the Go repository
//! (`testdata/golden/rips`, produced by running the real legacy model classes
//! with synthetic values). The expected files are the exact bytes Java wrote.

use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use tessia::phases::rips::encode::encode;
use tessia::phases::rips::model::Rips;

#[derive(Deserialize)]
struct Case {
    name: String,
    input: Rips,
    expected: String,
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rips/golden")
}

#[test]
fn encode_matches_every_java_golden_byte_for_byte() {
    let manifest = fs::read_to_string(golden_dir().join("manifest.json")).expect("manifest");
    let cases: Vec<Case> = serde_json::from_str(&manifest).expect("cases");
    assert!(cases.len() >= 4, "manifest has {} cases", cases.len());
    for case in cases {
        let want = fs::read_to_string(golden_dir().join(&case.expected)).expect("expected file");
        let got = encode(&case.input).expect("encode");
        assert_eq!(got, want, "case {}", case.name);
    }
}

#[test]
fn encode_does_not_mutate_its_input() {
    let manifest = fs::read_to_string(golden_dir().join("manifest.json")).expect("manifest");
    let cases: Vec<Case> = serde_json::from_str(&manifest).expect("cases");
    for case in cases {
        let before = case.input.clone();
        encode(&case.input).expect("encode");
        assert_eq!(case.input, before, "case {}", case.name);
    }
}
