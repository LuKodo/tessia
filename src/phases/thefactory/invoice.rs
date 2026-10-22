//! Work on one invoice: XML + PDF + ZIP and, at the same time, its CPScan part.
//!
//! Port of `downloadInvoice`, `fetchOrRead`, `fetchCPScan` and `processInvoice`
//! of Go `cmd_thefactory.go`. Files are written in a private [`Workdir`] and
//! published through [`Artifacts`] as they are produced (`<F>.xml`, `<F>.pdf`,
//! `XML/<F>_XML_PDF.zip`, `CPSCAN/<F>.support.<ext>`, then the sidecar last).

use anyhow::Result;
use chrono::{Datelike, Local, Timelike};
use rust_decimal::Decimal;

use super::client::{Kind, Status, TheFactoryClient};
use super::cpscan::{Acquirer, CpscanSidecar};
use super::plan::Plan;
use crate::context::AppContext;
use crate::fevxml;
use crate::storage::{Artifacts, Workdir};
use crate::stores::valor_xml::XmlValueUpsert;

/// Everything the workers share.
pub(crate) struct Env<'a> {
    pub ctx: &'a AppContext,
    pub artifacts: &'a Artifacts,
    pub cargue_id: &'a str,
    pub client: &'a TheFactoryClient,
    pub acquirer: &'a Acquirer,
}

/// The result of one invoice (the XML/PDF/ZIP outcome plus the CPScan note).
#[derive(Debug, Clone)]
pub(crate) struct Processed {
    /// `FEV1 XML=... PDF=... ZIP=... [(details)] [CPSCAN=...]`.
    pub line: String,
    /// XML, PDF and ZIP are all present.
    pub complete: bool,
    /// `CPSCAN sin soporte (...)` / `CPSCAN pendiente (...): ...` or `""`.
    pub warning: String,
}

/// Runs the XML/PDF/ZIP download and, concurrently, the CPScan lookup.
pub(crate) fn process(env: &Env<'_>, plan: &Plan) -> Result<Processed> {
    let workdir = env.ctx.workdir(env.cargue_id)?;
    let (downloaded, cpscan) = std::thread::scope(|scope| {
        let cpscan = plan
            .ssc
            .as_deref()
            .filter(|_| plan.wants_cpscan())
            .map(|ssc| scope.spawn(|| fetch_cpscan(env, &workdir, &plan.invoice, ssc)));
        let downloaded = download_invoice(env, &workdir, plan);
        let cpscan = cpscan.map(|handle| {
            handle.join().unwrap_or_else(|_| {
                (
                    "CPSCAN=PENDIENTE".to_owned(),
                    format!(
                        "CPSCAN pendiente (SSC {}): error interno",
                        plan.ssc.as_deref().unwrap_or_default()
                    ),
                )
            })
        });
        (downloaded, cpscan)
    });
    let (mut line, complete) = downloaded?;
    let mut warning = String::new();
    if let Some((part, note)) = cpscan {
        line.push(' ');
        line.push_str(&part);
        warning = note;
    }
    Ok(Processed {
        line,
        complete,
        warning,
    })
}

/// What `fetchOrRead` produced for one document.
struct Part {
    /// The usable bytes (`None`: the document is not usable).
    data: Option<Vec<u8>>,
    status: String,
    detail: Option<String>,
}

/// Reads the existing artifact or downloads and checks the document.
fn fetch_or_read(
    env: &Env<'_>,
    kind: Kind,
    invoice: &str,
    rel: &str,
    exists: bool,
    check: impl Fn(&[u8]) -> Result<Vec<u8>, String>,
) -> Result<Part> {
    if exists {
        return Ok(Part {
            data: Some(env.artifacts.fetch_bytes(env.cargue_id, rel)?),
            status: "YA_EXISTE".into(),
            detail: None,
        });
    }
    let result = env.client.fetch(kind, invoice)?;
    if result.status != Status::Downloaded {
        return Ok(Part {
            data: None,
            status: result.status.as_str().into(),
            detail: Some(result.detail),
        });
    }
    match check(&result.data) {
        Ok(data) => Ok(Part {
            data: Some(data),
            status: Status::Downloaded.as_str().into(),
            detail: None,
        }),
        Err(message) => Ok(Part {
            data: None,
            status: match kind {
                Kind::Pdf => "PDF_INVALIDO".into(),
                Kind::Xml => Status::Failed.as_str().into(),
            },
            detail: Some(message),
        }),
    }
}

fn check_xml(bytes: &[u8]) -> Result<Vec<u8>, String> {
    fevxml::validate(bytes).map_err(|e| e.to_string())?;
    Ok(fevxml::normalize_download(bytes))
}

/// `invoicefs.ValidPDF`: at least 1024 bytes and a `%PDF-` header.
pub(crate) fn check_pdf(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() < 1024 {
        return Err(format!(
            "invoicefs: no es un PDF: {} bytes (< 1024)",
            bytes.len()
        ));
    }
    let head = &bytes[..1024];
    let start = head
        .iter()
        .position(|b| !matches!(b, 0 | b'\t' | b'\r' | b'\n' | b' '))
        .unwrap_or(head.len());
    if !head[start..].starts_with(b"%PDF-") {
        return Err("invoicefs: no es un PDF: sin cabecera %PDF-".to_owned());
    }
    Ok(bytes.to_vec())
}

/// Writes `data` in the workdir and publishes it.
fn put(env: &Env<'_>, workdir: &Workdir, rel: &str, data: &[u8]) -> Result<()> {
    let path = workdir.write(rel, data)?;
    env.artifacts.publish_file(env.cargue_id, rel, &path)?;
    Ok(())
}

/// Mirrors `DescargarXML` + `DescargarPDF` + the XML/PDF ZIP for one invoice.
/// Returns the report line and whether the three files are present afterwards.
fn download_invoice(env: &Env<'_>, workdir: &Workdir, plan: &Plan) -> Result<(String, bool)> {
    let inv = plan.invoice.as_str();
    let (xml_rel, pdf_rel) = (format!("{inv}.xml"), format!("{inv}.pdf"));
    let xml = fetch_or_read(env, Kind::Xml, inv, &xml_rel, plan.has_xml, check_xml)?;
    let pdf = fetch_or_read(env, Kind::Pdf, inv, &pdf_rel, plan.has_pdf, check_pdf)?;
    let mut parts = vec![format!("XML={}", xml.status), format!("PDF={}", pdf.status)];
    let details: Vec<&str> = [&xml, &pdf]
        .iter()
        .filter_map(|p| p.detail.as_deref())
        .collect();

    if xml.status == Status::Downloaded.as_str() {
        put(
            env,
            workdir,
            &xml_rel,
            xml.data.as_deref().unwrap_or_default(),
        )?;
    }
    if pdf.status == Status::Downloaded.as_str() {
        put(
            env,
            workdir,
            &pdf_rel,
            pdf.data.as_deref().unwrap_or_default(),
        )?;
    }
    let mut zip_ok = plan.has_zip;
    let mut zip_len = None;
    match (&xml.data, &pdf.data) {
        _ if plan.has_zip => parts.push("ZIP=YA_EXISTE".into()),
        (Some(xml_bytes), Some(pdf_bytes)) => {
            let zip = fevxml::build_xml_pdf_zip(
                &format!("{inv}.pdf"),
                pdf_bytes,
                &format!("{inv}.xml"),
                xml_bytes,
                zip_now(),
            )?;
            put(env, workdir, &zip_rel(inv), &zip)?;
            zip_len = Some(zip.len());
            zip_ok = true;
            parts.push("ZIP=OK".into());
        }
        _ => parts.push("ZIP=FALTA".into()),
    }
    let mut line = format!("{inv} {}", parts.join(" "));
    if !details.is_empty() {
        line.push_str(&format!(" ({})", details.join("; ")));
    }
    let complete = xml.data.is_some() && pdf.data.is_some() && zip_ok;
    if complete {
        persist_xml_value(env, inv, xml.data.as_deref(), zip_len);
    }
    Ok((line, complete))
}

fn zip_rel(invoice: &str) -> String {
    format!("XML/{invoice}_XML_PDF.zip")
}

/// The current local time as a ZIP timestamp (`time.Now()` in Go).
fn zip_now() -> Option<zip::DateTime> {
    let now = Local::now();
    zip::DateTime::from_date_and_time(
        u16::try_from(now.year()).ok()?,
        u8::try_from(now.month()).ok()?,
        u8::try_from(now.day()).ok()?,
        u8::try_from(now.hour()).ok()?,
        u8::try_from(now.minute()).ok()?,
        u8::try_from(now.second()).ok()?,
    )
    .ok()
}

/// Keeps the XML total of a completed invoice in `factura_valor_xml` (tracking
/// columns). Best effort: it is never part of the phase outcome.
fn persist_xml_value(env: &Env<'_>, invoice: &str, xml: Option<&[u8]>, zip_len: Option<usize>) {
    let Some(xml) = xml else { return };
    let amount: Option<Decimal> = fevxml::read_total(xml);
    let value = XmlValueUpsert {
        invoice: invoice.to_owned(),
        amount,
        mtime_ms: chrono::Utc::now().timestamp_millis(),
        size: i64::try_from(zip_len.unwrap_or(xml.len())).unwrap_or(i64::MAX),
    };
    if let Err(err) = env.ctx.valor_xml().upsert_xml_values(&[value]) {
        tracing::warn!(invoice, error = %err, "xml total not stored");
    }
}

/// Looks the SSC up in CPScan, downloads its support and publishes
/// `CPSCAN/<F>.support.<ext>` and then the sidecar (the commit marker: a run
/// interrupted between both leaves the part pending). Never fails the invoice:
/// returns the `CPSCAN=` status for the report line and the warning for the
/// motivo (`""` on a plain success).
fn fetch_cpscan(env: &Env<'_>, workdir: &Workdir, invoice: &str, ssc: &str) -> (String, String) {
    let pending = |reason: &dyn std::fmt::Display| {
        (
            "CPSCAN=PENDIENTE".to_owned(),
            format!("CPSCAN pendiente (SSC {ssc}): {reason}"),
        )
    };
    let fetched = match env.acquirer.fetch(ssc) {
        Ok(fetched) => fetched,
        Err(err) => return pending(&format!("{err:#}")),
    };
    let sidecar = &fetched.sidecar;
    if sidecar.has_file() {
        let rel = CpscanSidecar::payload_path(invoice, &sidecar.ext);
        if let Err(err) = put(env, workdir, &rel, &fetched.payload) {
            return pending(&format!("{err:#}"));
        }
    }
    let written = sidecar
        .to_json()
        .and_then(|json| put(env, workdir, &CpscanSidecar::sidecar_path(invoice), &json));
    if let Err(err) = written {
        return pending(&format!("{err:#}"));
    }
    match sidecar.no_support_warning() {
        Some(warning) => ("CPSCAN=SIN_SOPORTE".to_owned(), warning),
        None => (
            format!("CPSCAN={}", sidecar.ext.to_uppercase()),
            String::new(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_check_requires_size_and_header() {
        let mut pdf = b"%PDF-1.4\n".to_vec();
        pdf.resize(2000, b'x');
        assert!(check_pdf(&pdf).is_ok());
        let mut padded = b"\x00\r\n  %PDF-1.7".to_vec();
        padded.resize(1500, b' ');
        assert!(check_pdf(&padded).is_ok(), "leading whitespace is allowed");
        assert_eq!(
            check_pdf(b"%PDF-1.4").unwrap_err(),
            "invoicefs: no es un PDF: 8 bytes (< 1024)"
        );
        assert_eq!(
            check_pdf(&vec![b'x'; 2000]).unwrap_err(),
            "invoicefs: no es un PDF: sin cabecera %PDF-"
        );
        let mut exact = b"%PDF-".to_vec();
        exact.resize(1024, b'a');
        assert!(check_pdf(&exact).is_ok());
        exact.pop();
        assert!(check_pdf(&exact).is_err());
    }

    #[test]
    fn xml_check_validates_and_cleans() {
        assert!(check_xml(b"<a/>").is_err());
        assert!(check_xml(b"<a><b></a> padding padding padding padding padding").is_err());
        let ok = check_xml(b"<a>padding padding padding padding padding padding</a>").expect("xml");
        assert!(ok.starts_with(b"<a>"));
    }

    #[test]
    fn zip_stamp_is_a_valid_dos_date() {
        assert!(zip_now().is_some());
    }
}
