//! Artifact IO of `conciliar`: the invoice XML, the CUV precheck of `<F>.txt`
//! and the clean-up of stale outputs (CONC-08, CONC-11).
//!
//! Go read and deleted files of the load folder; here the same relative names
//! live in the bucket, indexed by `artefactos` (flat names at the root of the
//! cargue, the XML ZIP under `XML/` or the older `ZIP/`).

use anyhow::Result;
use serde_json::Value;

use crate::fevxml::first_xml_entry;
use crate::storage::Artifacts;

/// Relative paths where `<F>_XML_PDF.zip` may be published (new place first).
pub fn xml_zip_paths(invoice: &str) -> [String; 2] {
    [
        format!("XML/{invoice}_XML_PDF.zip"),
        format!("ZIP/{invoice}_XML_PDF.zip"),
    ]
}

/// The invoice XML: the first `.xml` entry of the first non-empty
/// `<F>_XML_PDF.zip`, else a non-empty loose `<F>.xml`; `None` when there is
/// none (Go `invoiceXML`).
pub fn read_invoice_xml(
    artifacts: &Artifacts,
    cargue: &str,
    invoice: &str,
) -> Result<Option<Vec<u8>>> {
    let mut content = None;
    for rel in xml_zip_paths(invoice) {
        if !artifacts.is_published(cargue, &rel)? {
            continue;
        }
        let zip = artifacts.fetch_bytes(cargue, &rel)?;
        if zip.is_empty() {
            continue;
        }
        // A ZIP that cannot be read falls back to the loose file, as in Go.
        content = first_xml_entry(&zip).ok().flatten();
        break;
    }
    if content.is_none() {
        let rel = format!("{invoice}.xml");
        if artifacts.is_published(cargue, &rel)? {
            let bytes = artifacts.fetch_bytes(cargue, &rel)?;
            content = (!bytes.is_empty()).then_some(bytes);
        }
    }
    Ok(content)
}

/// Whether `<F>.txt` (the CUV response) already carries a CUV.
pub fn txt_has_cuv_published(artifacts: &Artifacts, cargue: &str, invoice: &str) -> Result<bool> {
    let rel = format!("{invoice}.txt");
    if !artifacts.is_published(cargue, &rel)? {
        return Ok(false);
    }
    Ok(txt_has_cuv(&artifacts.fetch_bytes(cargue, &rel)?))
}

/// Outputs that hold the old values and are removed once an invoice is
/// reconciled (Java `limpiarJsonCuvObsoletos`).
pub fn stale_names(invoice: &str) -> [String; 5] {
    [
        format!("{invoice}.json"),
        format!("{invoice}.txt"),
        format!("{invoice}_ERROR.txt"),
        format!("{invoice}_RESPUESTA_NO_CORRESPONDE.txt"),
        format!("{invoice}_VALOR_DIFERENTE.txt"),
    ]
}

/// Deletes the stale outputs of `invoice` from the bucket and the index;
/// returns the ones that existed.
pub fn remove_stale(artifacts: &Artifacts, cargue: &str, invoice: &str) -> Result<Vec<String>> {
    let mut removed = Vec::new();
    for rel in stale_names(invoice) {
        if artifacts.delete(cargue, &rel)? {
            removed.push(rel);
        }
    }
    Ok(removed)
}

fn valid_cuv(s: &str) -> bool {
    let s = s.trim();
    let upper = s.to_uppercase();
    !s.is_empty() && s != "-" && upper != "NULL" && !upper.contains("NO APLICA")
}

fn is_cuv_key(k: &str) -> bool {
    [
        "CodigoUnicoValidacion",
        "CodigoUnicoValidacionToShow",
        "CUV",
    ]
    .iter()
    .any(|n| n.eq_ignore_ascii_case(k))
}

fn json_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn find_cuv(v: &Value) -> Option<String> {
    match v {
        Value::Object(map) => map
            .iter()
            .filter(|(k, _)| is_cuv_key(k))
            .map(|(_, val)| json_text(val))
            .find(|s| valid_cuv(s))
            .map(|s| s.trim().to_owned())
            .or_else(|| map.values().find_map(find_cuv)),
        Value::Array(items) => items.iter().find_map(find_cuv),
        _ => None,
    }
}

/// Whether a CUV TXT (the ministry JSON response) carries a valid CUV, or
/// states that the CUV already exists or was validated (Go `TXTHasCUV`; the
/// radicacion port has the same rule).
pub fn txt_has_cuv(content: &[u8]) -> bool {
    let Ok(v) = serde_json::from_slice::<Value>(content) else {
        return false;
    };
    if find_cuv(&v).is_some() {
        return true;
    }
    let text = String::from_utf8_lossy(content).to_uppercase();
    text.contains("CUV")
        && ["YA SE ENCUENTRA", "YA EXISTE", "VALIDADO"]
            .iter()
            .any(|p| text.contains(p))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::db::test_support::TestDb;
    use crate::fevxml::build_xml_pdf_zip;
    use crate::storage::MemoryStore;

    const XML: &[u8] = b"<Invoice><cbc:ID>F1</cbc:ID></Invoice>";

    fn artifacts(db: &TestDb) -> Artifacts {
        Artifacts::new(
            Arc::new(MemoryStore::new()),
            crate::stores::artefactos::ArtefactosStore::new(db.pool()),
        )
    }

    #[test]
    fn cuv_detection_follows_the_go_rules() {
        for (json, want) in [
            (r#"{"CodigoUnicoValidacion":"abc123"}"#, true),
            (r#"{"x":[{"cuv":"abc"}]}"#, true),
            (r#"{"ResultState":false,"CUV":"-"}"#, false),
            (r#"{"CUV":"NO APLICA"}"#, false),
            (r#"{"CUV":null}"#, false),
            (r#"{"Mensaje":"El CUV ya existe"}"#, true),
            (r#"{"Mensaje":"documento validado sin cuv"}"#, true),
            (r#"{"Mensaje":"rechazado"}"#, false),
            ("not json, CUV YA EXISTE", false),
        ] {
            assert_eq!(txt_has_cuv(json.as_bytes()), want, "{json}");
        }
    }

    #[test]
    fn xml_comes_from_the_zip_first_then_the_loose_file() {
        let db = TestDb::new();
        let a = artifacts(&db);
        assert_eq!(read_invoice_xml(&a, "C1", "F1").expect("read"), None);
        a.publish_bytes("C1", "F1.xml", b"loose").expect("loose");
        assert_eq!(
            read_invoice_xml(&a, "C1", "F1").expect("read").as_deref(),
            Some(&b"loose"[..])
        );
        let zip = build_xml_pdf_zip("F1.pdf", b"%PDF", "F1.xml", XML, None).expect("zip");
        a.publish_bytes("C1", "ZIP/F1_XML_PDF.zip", &zip)
            .expect("old zip");
        assert_eq!(
            read_invoice_xml(&a, "C1", "F1").expect("read").as_deref(),
            Some(XML)
        );
        // An unreadable ZIP falls back to the loose file.
        a.publish_bytes("C1", "XML/F1_XML_PDF.zip", b"not a zip")
            .expect("bad zip");
        assert_eq!(
            read_invoice_xml(&a, "C1", "F1").expect("read").as_deref(),
            Some(&b"loose"[..])
        );
    }

    #[test]
    fn stale_outputs_are_removed_from_bucket_and_index() {
        let db = TestDb::new();
        let a = artifacts(&db);
        for name in stale_names("F1") {
            a.publish_bytes("C1", &name, b"x").expect("publish");
        }
        a.publish_bytes("C1", "F2.json", b"keep")
            .expect("other invoice");
        a.publish_bytes("C1", "XML/F1_XML_PDF.zip", b"keep")
            .expect("zip");
        let removed = remove_stale(&a, "C1", "F1").expect("remove");
        assert_eq!(removed.len(), 5);
        assert_eq!(
            remove_stale(&a, "C1", "F1").expect("again"),
            Vec::<String>::new()
        );
        let left: Vec<_> = a
            .list("C1")
            .expect("list")
            .into_iter()
            .map(|r| r.rel_path)
            .collect();
        assert_eq!(left, ["F2.json", "XML/F1_XML_PDF.zip"]);
    }

    #[test]
    fn existing_cuv_is_read_from_the_published_txt() {
        let db = TestDb::new();
        let a = artifacts(&db);
        assert!(!txt_has_cuv_published(&a, "C1", "F1").expect("none"));
        a.publish_bytes("C1", "F1.txt", br#"{"CUV":"-"}"#)
            .expect("txt");
        assert!(!txt_has_cuv_published(&a, "C1", "F1").expect("no cuv"));
        a.publish_bytes("C1", "F1.txt", br#"{"CUV":"deadbeef"}"#)
            .expect("txt");
        assert!(txt_has_cuv_published(&a, "C1", "F1").expect("cuv"));
    }
}
