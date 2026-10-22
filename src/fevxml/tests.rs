//! Golden-style tests built from the (synthetic) Go goldens, copied under
//! `tests/fixtures/fevxml/`.

use std::path::PathBuf;

use rust_decimal::Decimal;
use serde_json::Value;

use super::*;

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/fevxml")
        .join(rel)
}

fn read(rel: &str) -> Vec<u8> {
    std::fs::read(fixture(rel)).unwrap()
}

fn manifest(rel: &str) -> Value {
    serde_json::from_slice(&read(rel)).unwrap()
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

fn assert_bytes(got: &[u8], want: &[u8]) {
    if got != want {
        let i = got.iter().zip(want).take_while(|(a, b)| a == b).count();
        let w = |b: &[u8]| String::from_utf8_lossy(&b[i..b.len().min(i + 120)]).into_owned();
        panic!(
            "differs at byte {i}:\n got: {:?}\nwant: {:?}",
            w(got),
            w(want)
        );
    }
}

fn dec(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn params(c: &Value) -> Params {
    Params {
        codigo_prestador: "1300185002".into(),
        contrato: s(c, "contrato").into(),
        regimen: s(c, "regimen").into(),
        periodo_inicio: PERIODO_INICIO_MINISTERIO.into(),
    }
}

#[test]
fn golden_download() {
    let m = manifest("fev/manifest.json");
    for c in m["xml"].as_array().unwrap() {
        let got = normalize_download(&read(&format!("fev/{}", s(c, "input"))));
        assert_bytes(&got, &read(&format!("fev/{}", s(c, "download"))));
    }
}

#[test]
fn golden_ministerio() {
    let m = manifest("fev/manifest.json");
    for c in m["xml"].as_array().unwrap() {
        let input = read(&format!("fev/{}", s(c, "input")));
        let res = prepare_ministerio(&input, &params(c));
        if !c["contrato_existe"].as_bool().unwrap() {
            assert!(
                matches!(res, Err(FevxmlError::SinContrato)),
                "{}",
                s(c, "name")
            );
            continue;
        }
        let got = res.unwrap();
        if c["error"].is_null() {
            assert!(got.period_error.is_none(), "{}", s(c, "name"));
        } else {
            assert!(matches!(got.period_error, Some(FevxmlError::Periodo)));
        }
        let want = read(&format!("fev/{}", s(c, "ministerio")));
        assert_bytes(&got.xml, &want);
        assert_eq!(got.changed, input != want, "{}", s(c, "name"));
    }
}

/// Java is not idempotent (FACTURA_SIN_CONTRATO is removed and re-inserted,
/// leaving whitespace); a re-send must match it.
#[test]
fn golden_ministerio_second_pass() {
    let m = manifest("fev/manifest.json");
    let mut n = 0;
    for c in m["xml"].as_array().unwrap() {
        let Some(second) = c["ministerio2"].as_str() else {
            continue;
        };
        n += 1;
        let first = read(&format!("fev/{}", s(c, "ministerio")));
        let got = prepare_ministerio(&first, &params(c)).unwrap();
        assert!(got.period_error.is_none());
        assert_bytes(&got.xml, &read(&format!("fev/{second}")));
    }
    assert!(n >= 3);
}

#[test]
fn ministerio_without_contract_edits_nothing() {
    let input = read("fev/xml/sin_contrato.input.xml");
    let err = prepare_ministerio(&input, &Params::default()).unwrap_err();
    assert_eq!(err.reason_code(), Some("XML_SIN_CONTRATO"));
    assert!(err.to_string().contains("NUMERO_CONTRATO"));
}

#[test]
fn validate_rules() {
    validate(&read("fev/xml/contributivo.input.xml")).unwrap();
    assert!(matches!(validate(b"<a/>"), Err(FevxmlError::XmlCorto(4))));
    let bad = format!("<?xml version=\"1.0\"?><a><b></a>{}", " ".repeat(20));
    assert!(matches!(
        validate(bad.as_bytes()),
        Err(FevxmlError::XmlMalFormado(_))
    ));
    let no_root = format!("<?xml version=\"1.0\"?><!-- x -->{}", " ".repeat(30));
    assert!(matches!(
        validate(no_root.as_bytes()),
        Err(FevxmlError::XmlSinRaiz)
    ));
}

#[test]
fn golden_cucon_and_cuota() {
    let m = manifest("fevajuste/manifest.json");
    let cucon_value = s(&m, "cucon_valor");
    for c in m["xml"].as_array().unwrap() {
        let name = s(c, "name");
        let input = read(&format!("fevajuste/{}", s(c, "input")));

        let (got, res) = cucon(&input, cucon_value);
        assert_eq!(res.as_str(), s(c, "cucon_resultado"), "{name}");
        assert_bytes(&got, &read(&format!("fevajuste/{}", s(c, "cucon"))));

        let r = cuota_moderadora(&input, "<ARCHIVO>");
        assert_eq!(
            r.changed,
            c["cuota_modificado"].as_bool().unwrap(),
            "{name}"
        );
        let warnings: String = r.warnings.iter().map(|w| format!("{w}\n")).collect();
        assert_eq!(warnings, s(c, "cuota_salida"), "{name}");
        assert_bytes(&r.xml, &read(&format!("fevajuste/{}", s(c, "cuota"))));
    }
}

#[test]
fn golden_total() {
    let m = manifest("fevajuste/manifest.json");
    for c in m["total"].as_array().unwrap() {
        let got = read_total(s(c, "xml").as_bytes());
        match c["total"].as_str() {
            None => assert!(got.is_none(), "{}", s(c, "name")),
            Some(want) => assert_eq!(got.unwrap().to_string(), want, "{}", s(c, "name")),
        }
    }
}

#[test]
fn golden_numero_convenio() {
    let m = manifest("fevajuste/manifest.json");
    for c in m["numero_convenio"].as_array().unwrap() {
        let got = numero_convenio(s(c, "input"));
        match c["value"].as_str() {
            None => assert!(got.is_none(), "{:?}", s(c, "input")),
            Some(want) => assert_eq!(got.unwrap().to_string(), want, "{:?}", s(c, "input")),
        }
    }
}

/// The cuota tool only touches XMLs with an anticipo to legalize; the download
/// edits already removed ANTICIPOS.
#[test]
fn cuota_after_download_is_stable() {
    let input = read("fevajuste/xml/anticipo.cuota.xml");
    let r = cuota_moderadora(&input, "x");
    assert_bytes(&r.xml, &input);
    assert!(String::from_utf8_lossy(&input).contains("schemeID=\"04\""));
}

#[test]
fn cucon_with_legacy_value_reports_modificado_with_same_bytes() {
    let input = b"<AdditionalInformation><Name>NUMERO_CONTRATO</Name><Value>22382</Value></AdditionalInformation>";
    let (got, res) = cucon(input, "22382");
    assert_eq!(res, ContratoResult::Modificado);
    assert_bytes(&got, input);
}

#[test]
fn invoice_lines_from_attached_document() {
    let got = invoice_lines(&read("conciliacion/attached_3lineas.xml")).unwrap();
    let line = |id: &str, d: &str, q: &str, p: &str, t: &str| InvoiceLine {
        identifier: id.into(),
        description: d.into(),
        quantity: dec(q),
        unit_price: dec(p),
        line_total: dec(t),
    };
    let want = [
        line(
            "23_11_19-27363-1",
            "PRODUCTO SINTETICO UNO TABLETA",
            "10",
            "235",
            "2350",
        ),
        line(
            "57_20_7-20021574-2",
            "PRODUCTO SINTETICO DOS JARABE",
            "3",
            "333.5",
            "1000.5",
        ),
        line(
            "300_1_1-12345",
            "INSUMO SINTETICO GASA",
            "2",
            "349.75",
            "699.5",
        ),
    ];
    assert_eq!(got, want);
}

fn invoice(q: &str, p: &str, total: &str, id: &str) -> Vec<u8> {
    format!(
        r#"<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2" xmlns:cac="c" xmlns:cbc="b">
<cac:InvoiceLine><cbc:InvoicedQuantity>{q}</cbc:InvoicedQuantity><cbc:LineExtensionAmount>{total}</cbc:LineExtensionAmount>
<cac:Item><cbc:Description>X</cbc:Description><cac:StandardItemIdentification><cbc:ID>{id}</cbc:ID></cac:StandardItemIdentification></cac:Item>
<cac:Price><cbc:PriceAmount>{p}</cbc:PriceAmount></cac:Price></cac:InvoiceLine></Invoice>"#
    )
    .into_bytes()
}

#[test]
fn invoice_lines_plain_invoice_and_errors() {
    let got = invoice_lines(&invoice("2", "10", "20", "1_1-123")).unwrap();
    assert_eq!((got.len(), got[0].line_total), (1, dec("20")));

    assert!(matches!(
        invoice_lines(br#"<Invoice><cbc:ID xmlns:cbc="b">1</cbc:ID></Invoice>"#),
        Err(FevxmlError::SinLineas)
    ));
    assert!(matches!(
        invoice_lines(b"<<<"),
        Err(FevxmlError::XmlInvalido(_))
    ));
    assert!(matches!(
        invoice_lines(&invoice("0", "10", "0", "1_1-123")),
        Err(FevxmlError::LineaIncompleta(_))
    ));
    assert!(matches!(
        invoice_lines(&invoice("2", "10", "20", "")),
        Err(FevxmlError::LineaIncompleta(_))
    ));
    let err = invoice_lines(&invoice("2", "10", "20.02", "1_1-123")).unwrap_err();
    assert!(matches!(err, FevxmlError::LineaInconsistente(ref id) if id == "1_1-123"));
    assert_eq!(
        err.to_string(),
        "la cantidad y el precio XML no forman el total de la linea 1_1-123"
    );
    // Within the 0.01 tolerance of the PHP check.
    invoice_lines(&invoice("3", "333.33", "1000", "1_1-123")).unwrap();
}

#[test]
fn format_and_storage_amounts() {
    for (input, display, storage) in [
        ("1,234.50", "1234.5", "1234.50"),
        ("700.00", "700", "700.00"),
        ("0", "0", "0.00"),
        ("12.345", "12.345", "12.35"),
        ("1000", "1000", "1000.00"),
    ] {
        let xml = format!(
            "<AttachedDocument><cbc:Description><![CDATA[<Invoice><cac:LegalMonetaryTotal><cbc:PayableAmount currencyID=\"COP\">{input}</cbc:PayableAmount></cac:LegalMonetaryTotal></Invoice>]]></cbc:Description></AttachedDocument>"
        );
        let d = read_total(xml.as_bytes()).unwrap();
        assert_eq!(format_total(d), display, "{input}");
        assert_eq!(storage_amount(d), storage, "{input}");
    }
}

fn attached(amount: &str) -> Vec<u8> {
    format!(
        "<AttachedDocument xmlns:cac=\"c\" xmlns:cbc=\"b\"><cbc:Description><![CDATA[<Invoice><cac:LegalMonetaryTotal><cbc:PayableAmount currencyID=\"COP\">{amount}</cbc:PayableAmount></cac:LegalMonetaryTotal></Invoice>]]></cbc:Description></AttachedDocument>"
    )
    .into_bytes()
}

fn write(path: &std::path::Path, data: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, data).unwrap();
}

#[test]
fn invoice_total_rules() {
    let zip_with = |entries: &[(&str, &[u8])]| {
        // Build through the public helpers: the PDF first, then the last entry.
        let (name, data) = entries[entries.len() - 1];
        build_xml_pdf_zip("F1.pdf", b"%PDF", name, data, None).unwrap()
    };
    let check = |setup: &dyn Fn(&std::path::Path), state: XmlState, display: &str, is_zip: bool| {
        let dir = tempfile::tempdir().unwrap();
        setup(dir.path());
        let (file, total, got) = invoice_total(dir.path(), "F1");
        assert_eq!(got, state);
        if state == XmlState::Missing {
            assert!(file.is_none());
            return;
        }
        let f = file.unwrap();
        assert_eq!((f.is_zip, f.size > 0), (is_zip, true));
        if state == XmlState::Found {
            assert_eq!(format_total(total.unwrap()), display);
        }
    };
    check(
        &|d| {
            write(
                &d.join("XML/F1_XML_PDF.zip"),
                &zip_with(&[("F1.xml", &attached("1,234.50"))]),
            )
        },
        XmlState::Found,
        "1234.5",
        true,
    );
    check(
        &|d| {
            write(
                &d.join("ZIP/F1_XML_PDF.zip"),
                &zip_with(&[("a.xml", &attached("700.00"))]),
            )
        },
        XmlState::Found,
        "700",
        true,
    );
    check(
        &|d| write(&d.join("F1.xml"), &attached("90000.10")),
        XmlState::Found,
        "90000.1",
        false,
    );
    check(
        &|d| {
            write(
                &d.join("XML/F1_XML_PDF.zip"),
                &zip_with(&[("a.xml", &attached("10"))]),
            );
            write(&d.join("F1.xml"), &attached("20"));
        },
        XmlState::Found,
        "10",
        true,
    );
    check(
        &|d| {
            write(&d.join("XML/F1_XML_PDF.zip"), b"");
            write(&d.join("F1.xml"), &attached("20"));
        },
        XmlState::Found,
        "20",
        false,
    );
    check(&|_| {}, XmlState::Missing, "", false);
    check(
        &|d| write(&d.join("F1.xml"), b""),
        XmlState::Missing,
        "",
        false,
    );
    check(
        &|d| write(&d.join("F1.xml"), b"<Invoice/>"),
        XmlState::Unparsable,
        "",
        false,
    );
    check(
        &|d| {
            // A ZIP without an xml entry does not fall back to the loose xml.
            let z = build_xml_pdf_zip("F1.pdf", b"%PDF", "F1.txt", b"x", None).unwrap();
            write(&d.join("XML/F1_XML_PDF.zip"), &z);
            write(&d.join("F1.xml"), &attached("20"));
        },
        XmlState::Unparsable,
        "",
        true,
    );
    check(
        &|d| write(&d.join("XML/F1_XML_PDF.zip"), b"not a zip"),
        XmlState::Unparsable,
        "",
        true,
    );
}

#[test]
fn zip_path_helpers() {
    let paths = xml_pdf_zip_paths(std::path::Path::new("w"), "F1");
    assert_eq!(paths[0], std::path::Path::new("w/XML/F1_XML_PDF.zip"));
    assert_eq!(paths[1], std::path::Path::new("w/ZIP/F1_XML_PDF.zip"));
}
