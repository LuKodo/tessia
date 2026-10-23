//! Every branch of the generator with synthetic inputs: page order, pages and
//! result codes.

#[path = "fixtures/soporte/common.rs"]
mod soporte_common;

use anyhow::anyhow;
use image::ImageFormat;
use soporte_common::*;
use tessia::phases::soporte::cpscan::Stored;
use tessia::phases::soporte::generator::{ManualSignature, Request, build};
use tessia::phases::soporte::input::{Header, Item};
use tessia::phases::soporte::render::merge::{page_count, page_texts};
use tessia::phases::soporte::render::signature;

fn header(ceco: &str, items: usize) -> Header {
    Header {
        invoice: "FEV1".into(),
        ssc: "10".into(),
        cost_centre: ceco.into(),
        authorization: "AUT".into(),
        doc_type: "CC".into(),
        doc_number: "123".into(),
        name: "ANA PEREZ".into(),
        date: "2026-10-05".into(),
        items: (1..=items)
            .map(|i| Item {
                article: format!("ART {i}"),
                quantity: "1".into(),
                date: "2026-10-05".into(),
                authorization: "AUT".into(),
            })
            .collect(),
        other_ssc_lines: 0,
    }
}

fn stored(origin: &str, additional: &str, file: &str, ext: &str) -> Stored {
    Stored {
        version: 1,
        ssc: "10".into(),
        found: true,
        origin: origin.into(),
        additional: additional.into(),
        file: file.into(),
        ext: ext.into(),
        ..Stored::default()
    }
}

fn manual(_: &Header) -> anyhow::Result<ManualSignature> {
    Ok(ManualSignature {
        png: signature::render("ANA PEREZ")?,
        generated: true,
        note: String::new(),
    })
}

struct Fixture {
    header: Header,
    stored: Option<Stored>,
    payload: Vec<u8>,
    invoice_pdf: Vec<u8>,
    signature: Option<Vec<u8>>,
    logo: Result<Vec<u8>, String>,
}

impl Fixture {
    fn new(ceco: &str, stored: Option<Stored>, payload: Vec<u8>) -> Fixture {
        Fixture {
            header: header(ceco, 2),
            stored,
            payload,
            invoice_pdf: text_pdf("FACTURA", 2),
            signature: None,
            logo: Ok(rgb_image(300, 100, ImageFormat::Png)),
        }
    }

    fn run(&self) -> tessia::phases::soporte::generator::Built {
        let request = Request {
            header: &self.header,
            stored: self.stored.as_ref(),
            payload: &self.payload,
            problem: None,
            invoice_pdf: &self.invoice_pdf,
            signature: self.signature.as_deref(),
            logo: self.logo.as_deref().map_err(String::as_str),
        };
        build(&request, &manual)
    }
}

fn image() -> Vec<u8> {
    rgb_image(200, 100, ImageFormat::Jpeg)
}

/// A short label of each page: its first drawn text, or "image".
fn kinds(pdf: &[u8]) -> Vec<String> {
    page_texts(pdf)
        .expect("texts")
        .into_iter()
        .map(|t| {
            if t.is_empty() {
                "image".to_owned()
            } else if t.contains("CONSTANCIA DE ENTREGA") {
                "delivery".to_owned()
            } else {
                t.split(" page").next().unwrap_or_default().to_owned()
            }
        })
        .collect()
}

#[test]
fn pc_uses_the_scanned_pdf_as_is_followed_by_the_invoice() {
    let scan = text_pdf("ESCANEADO", 1);
    assert!(scan.len() >= 100);
    let f = Fixture::new(
        "CENTRO",
        Some(stored("PC", "http://x/a.pdf", "http://x/a.pdf", "pdf")),
        scan,
    );
    let built = f.run();
    assert_eq!(
        (built.code, built.branch.map(|b| b.as_str())),
        ("OK", Some("PC"))
    );
    assert_eq!(kinds(&built.pdf), ["ESCANEADO", "FACTURA", "FACTURA"]);
}

#[test]
fn a_tiny_pc_pdf_is_a_cpscan_problem() {
    let f = Fixture::new(
        "CENTRO",
        Some(stored("PC", "u", "u", "pdf")),
        b"%PDF-tiny".to_vec(),
    );
    let built = f.run();
    assert_eq!(built.code, "CPSCAN_NO_DESCARGADO");
    assert_eq!(built.detail, "PC: pdf soporte invalido (< 100 bytes)");
}

#[test]
fn app_without_a_signature_is_formula_plus_invoice() {
    let f = Fixture::new(
        "CENTRO",
        Some(stored("APP", "", "formula-10.jpg", "jpg")),
        image(),
    );
    let built = f.run();
    assert_eq!(built.branch.map(|b| b.as_str()), Some("APP"));
    assert_eq!(kinds(&built.pdf), ["image", "FACTURA", "FACTURA"]);
    assert!(built.detail.is_empty() && built.note.is_empty());
}

#[test]
fn app_with_a_signature_adds_the_delivery_page_between_formula_and_invoice() {
    let mut f = Fixture::new(
        "CENTRO",
        Some(stored("BUCKET", "", "formula-10.jpg", "jpg")),
        image(),
    );
    f.signature = Some(signature::render("ANA PEREZ").expect("signature"));
    let built = f.run();
    assert_eq!(built.branch.map(|b| b.as_str()), Some("APP"));
    assert_eq!(
        kinds(&built.pdf),
        ["image", "delivery", "FACTURA", "FACTURA"]
    );
    let texts = page_texts(&built.pdf).expect("texts");
    assert!(
        texts[1].contains("ART 1") && texts[1].contains("ART 2"),
        "every delivery is listed"
    );
}

#[test]
fn sl_is_like_app_with_its_own_branch() {
    let mut f = Fixture::new(
        "CENTRO",
        Some(stored("SL", "", "firma-10.png", "png")),
        rgb_image(50, 50, ImageFormat::Png),
    );
    f.signature = Some(signature::render("X").expect("signature"));
    let built = f.run();
    assert_eq!(built.branch.map(|b| b.as_str()), Some("SL"));
    assert_eq!(
        kinds(&built.pdf),
        ["image", "delivery", "FACTURA", "FACTURA"]
    );
}

#[test]
fn formula_fallback_for_unknown_origins_has_no_delivery_page_outside_deposits() {
    let mut f = Fixture::new(
        "CENTRO",
        Some(stored("XYZ", "", "formula-10.jpg", "jpg")),
        image(),
    );
    f.signature = Some(signature::render("X").expect("signature"));
    let built = f.run();
    assert_eq!(built.branch.map(|b| b.as_str()), Some("FORMULA"));
    assert_eq!(kinds(&built.pdf), ["image", "FACTURA", "FACTURA"]);
}

#[test]
fn a_deposit_that_needs_a_delivery_page_fails_with_the_missing_format() {
    let mut f = Fixture::new(
        "BLOC PORT",
        Some(stored("APP", "", "formula-10.jpg", "jpg")),
        image(),
    );
    f.signature = Some(signature::render("X").expect("signature"));
    let built = f.run();
    assert_eq!(built.code, "FORMATO_DEPOSITO_NO_DISPONIBLE");
    assert!(built.detail.contains("Formato_App_Deposito"));
    // Without a signature the deposit gets formula + invoice.
    f.signature = None;
    assert_eq!(kinds(&f.run().pdf), ["image", "FACTURA", "FACTURA"]);
}

#[test]
fn manual_builds_the_delivery_page_with_a_generated_signature_note() {
    let f = Fixture::new("CENTRO", Some(stored("APP", "", "", "")), Vec::new());
    let built = f.run();
    assert_eq!(
        (built.code, built.branch.map(|b| b.as_str())),
        ("OK", Some("MANUAL"))
    );
    assert_eq!(built.detail, "soporte manual: CPScan sin soporte");
    assert_eq!(built.note, "firma generada desde el nombre del paciente");
    assert_eq!(kinds(&built.pdf), ["delivery", "FACTURA", "FACTURA"]);
}

#[test]
fn manual_notes_other_ssc_lines_and_signature_remarks() {
    let mut f = Fixture::new("CENTRO", Some(stored("APP", "", "", "")), Vec::new());
    f.header.other_ssc_lines = 2;
    let built = f.run();
    assert_eq!(
        built.note,
        "firma generada desde el nombre del paciente; entrega: 2 linea(s) de otro SSC no se listaron (solo las del SSC 10)"
    );
}

#[test]
fn manual_failures_are_render_failures() {
    let f = Fixture::new("CENTRO", Some(stored("APP", "", "", "")), Vec::new());
    let request = Request {
        header: &f.header,
        stored: f.stored.as_ref(),
        payload: &[],
        problem: None,
        invoice_pdf: &f.invoice_pdf,
        signature: None,
        logo: Ok(&[]),
    };
    let built = build(&request, &|_| Err(anyhow!("boom")));
    assert_eq!(
        (built.code, built.detail.as_str()),
        ("RENDER_FALLIDO", "soporte manual: boom")
    );
    let built = build(&request, &manual);
    assert_eq!(built.code, "RENDER_FALLIDO");
    assert!(
        built.detail.starts_with("Formato_App: "),
        "{}",
        built.detail
    );
    let bad_logo = Request {
        logo: Err("supportrenderer: delivery logo is not a PNG or JPEG image"),
        ..request
    };
    let built = build(&bad_logo, &manual);
    assert!(
        built.detail.contains("delivery logo is not a PNG or JPEG"),
        "{}",
        built.detail
    );
}

#[test]
fn mandatory_deposits_and_sede_002_cannot_be_manual() {
    for ceco in ["BODEGA 1", "SEDE 002"] {
        let f = Fixture::new(ceco, Some(stored("APP", "", "", "")), Vec::new());
        assert_eq!(f.run().code, "SOPORTE_OBLIGATORIO_NO_DISPONIBLE", "{ceco}");
    }
}

#[test]
fn a_failed_delivery_page_keeps_formula_plus_invoice_and_says_so() {
    let mut f = Fixture::new(
        "CENTRO",
        Some(stored("APP", "", "formula-10.jpg", "jpg")),
        image(),
    );
    f.signature = Some(signature::render("X").expect("signature"));
    f.logo = Err("logo roto".into());
    let built = f.run();
    assert_eq!(built.code, "OK");
    assert_eq!(built.detail, "SOPORTE_NO_GENERADO: logo roto");
    assert_eq!(kinds(&built.pdf), ["image", "FACTURA", "FACTURA"]);
}

#[test]
fn missing_inputs_have_their_codes_and_hints() {
    let mut f = Fixture::new("CENTRO", None, Vec::new());
    let built = f.run();
    assert_eq!(built.code, "CPSCAN_NO_DESCARGADO");
    assert_eq!(
        built.detail,
        "falta CPSCAN/FEV1.cpscan.json (ejecute `facturacion thefactory descargar`)"
    );
    f.header.ssc.clear();
    assert_eq!(f.run().code, "SIN_SSC");
    f.header.ssc = "10".into();
    f.stored = Some(stored("APP", "", "formula-10.jpg", "jpg"));
    let built = f.run();
    assert_eq!(built.code, "CPSCAN_NO_DESCARGADO");
    assert!(
        built
            .detail
            .starts_with("falta el archivo CPSCAN/FEV1.support.jpg"),
        "{}",
        built.detail
    );
    f.payload = image();
    f.invoice_pdf = Vec::new();
    let built = f.run();
    assert_eq!(built.code, "FACTURA_PDF_FALTA");
    assert!(built.detail.starts_with("falta FEV1.pdf"));
    f.invoice_pdf = b"not a pdf".to_vec();
    assert_eq!(f.run().code, "UNION_PDF_FALLIDA");
    f.invoice_pdf = text_pdf("FACTURA", 1);
    f.payload = b"garbage".to_vec();
    let built = f.run();
    assert_eq!(built.code, "RENDER_FALLIDO");
    assert!(
        built.detail.starts_with("Formato_Formula: "),
        "{}",
        built.detail
    );
}

#[test]
fn page_counts_follow_the_parts() {
    let mut f = Fixture::new(
        "CENTRO",
        Some(stored("APP", "", "formula-10.jpg", "jpg")),
        image(),
    );
    f.signature = Some(signature::render("X").expect("signature"));
    f.header = header("CENTRO", 40);
    // formula (1) + delivery (3 pages for 40 rows) + invoice (2).
    assert_eq!(page_count(&f.run().pdf).expect("pages"), 6);
}
