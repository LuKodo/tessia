//! Builds the support PDF of one invoice (Go `Generator.Build`):
//! `[support page(s)] + invoice PDF`, over what `thefactory descargar` left in
//! `CPSCAN/`. It performs no I/O: every input is a parameter.

use anyhow::Result;

use super::cpscan::{MIN_PC_SUPPORT, Stored};
use super::decide::{
    Branch, CODE_CPSCAN_MISSING, CODE_DEPOSIT_FORMAT_MISSING, CODE_MERGE_FAILED,
    CODE_NO_INVOICE_PDF, CODE_OK, CODE_RENDER_FAILED, Decision, REPORT_DELIVERY_DEPOSIT, decide,
};
use super::input::Header;
use super::render::delivery::{self, DeliveryRow};
use super::render::{formula, merge};

/// Name of the delivery report, for messages.
pub const REPORT_DELIVERY: &str = "Formato_App";

/// The command that produces the CPScan files, for the Spanish hints.
const HINT: &str = "ejecute `facturacion thefactory descargar`";

/// The patient signature of the manual support.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualSignature {
    pub png: Vec<u8>,
    /// No signature file existed: the image was drawn from the patient name.
    pub generated: bool,
    /// A non-fatal remark (e.g. the generated image could not be saved).
    pub note: String,
}

/// One invoice to build.
pub struct Request<'a> {
    pub header: &'a Header,
    /// The CPScan lookup (`None`: never fetched, or unusable: see `problem`).
    pub stored: Option<&'a Stored>,
    /// Its support file.
    pub payload: &'a [u8],
    /// Why a sidecar that exists cannot be used.
    pub problem: Option<&'a str>,
    /// `<F>.pdf`; empty when missing.
    pub invoice_pdf: &'a [u8],
    /// Patient signature PNG of the CPScan-backed delivery page, or `None`
    /// (signatures disabled or no file). It only adds that page.
    pub signature: Option<&'a [u8]>,
    /// Logo of the delivery page, or why it cannot be used.
    pub logo: std::result::Result<&'a [u8], &'a str>,
}

/// The outcome for one invoice. `pdf` is set only when `code` is `OK`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Built {
    pub code: &'static str,
    pub detail: String,
    /// A non-fatal remark of a built support.
    pub note: String,
    pub branch: Option<Branch>,
    pub pdf: Vec<u8>,
}

fn fail(code: &'static str, detail: impl Into<String>) -> Built {
    Built {
        code,
        detail: detail.into(),
        note: String::new(),
        branch: None,
        pdf: Vec::new(),
    }
}

/// Assembles the support. `manual_signature` is only called by the `MANUAL` branch.
pub fn build(
    req: &Request<'_>,
    manual_signature: &dyn Fn(&Header) -> Result<ManualSignature>,
) -> Built {
    let header = req.header;
    let (branch, formula_report, delivery_page, deposit, mut detail) =
        match decide(header, req.stored) {
            Decision::Fail { code, detail } if code == CODE_CPSCAN_MISSING => {
                return fail(code, missing_cpscan(header, req.problem, detail));
            }
            Decision::Fail { code, detail } => return fail(code, detail),
            Decision::Build {
                branch,
                formula,
                delivery,
                deposit,
                detail,
            } => (branch, formula, delivery, deposit, detail),
        };
    if req.invoice_pdf.is_empty() {
        return fail(
            CODE_NO_INVOICE_PDF,
            format!("falta {}.pdf ({HINT})", header.invoice),
        );
    }
    let mut parts: Vec<Vec<u8>> = Vec::new();
    let mut note = String::new();
    let mut lists_deliveries = false;
    let has_file = req.stored.is_some_and(Stored::has_file);
    if branch == Branch::Manual {
        match manual_page(req, manual_signature) {
            Ok((page, signature)) => {
                parts.push(page);
                lists_deliveries = true;
                if signature.generated {
                    note = "firma generada desde el nombre del paciente".into();
                }
                note = join_note(note, &signature.note);
            }
            Err(failure) => return failure,
        }
    } else if has_file && req.payload.is_empty() {
        let ext = req.stored.map_or("", |s| s.ext.as_str());
        return fail(
            CODE_CPSCAN_MISSING,
            format!(
                "falta el archivo CPSCAN/{}.support.{ext} ({HINT})",
                header.invoice
            ),
        );
    } else if branch == Branch::Pc {
        if req.payload.len() < MIN_PC_SUPPORT {
            return fail(
                CODE_CPSCAN_MISSING,
                "PC: pdf soporte invalido (< 100 bytes)",
            );
        }
        parts.push(req.payload.to_vec());
    } else {
        if delivery_page && req.signature.is_some() && deposit {
            return fail(
                CODE_DEPOSIT_FORMAT_MISSING,
                format!("{REPORT_DELIVERY_DEPOSIT} no tiene renderer (pending-inputs #1)"),
            );
        }
        let report = formula_report.map_or("Formato_Formula", |r| r.name());
        match formula::render(req.payload) {
            Ok(page) => parts.push(page),
            Err(err) => return fail(CODE_RENDER_FAILED, format!("{report}: {err:#}")),
        }
        if let (true, Some(signature)) = (delivery_page, req.signature) {
            match render_delivery(header, req.logo, signature) {
                Ok(page) => {
                    parts.push(page);
                    lists_deliveries = true;
                }
                // Java: APP/SL_SOPORTE_NO_GENERADO, formula + invoice only.
                Err(err) => detail = format!("SOPORTE_NO_GENERADO: {err:#}"),
            }
        }
    }
    if header.other_ssc_lines > 0 && lists_deliveries {
        note = join_note(
            note,
            &format!(
                "entrega: {} linea(s) de otro SSC no se listaron (solo las del SSC {})",
                header.other_ssc_lines, header.ssc
            ),
        );
    }
    parts.push(req.invoice_pdf.to_vec());
    let refs: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
    match merge::merge(&refs) {
        Ok(pdf) => Built {
            code: CODE_OK,
            detail,
            note,
            branch: Some(branch),
            pdf,
        },
        Err(err) => fail(CODE_MERGE_FAILED, format!("{err:#}")),
    }
}

fn missing_cpscan(header: &Header, problem: Option<&str>, detail: String) -> String {
    match problem {
        Some(problem) => format!("{problem} ({HINT})"),
        None if detail.is_empty() => {
            format!("falta CPSCAN/{}.cpscan.json ({HINT})", header.invoice)
        }
        None => detail,
    }
}

fn join_note(a: String, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (_, true) => a,
        (true, false) => b.to_owned(),
        (false, false) => format!("{a}; {b}"),
    }
}

/// The delivery rows of a formula: one per delivery (`incremento` 1..N, the
/// line's own article, quantity, date and authorization; the patient fields
/// repeat the first line's).
pub fn delivery_rows(header: &Header) -> Vec<DeliveryRow> {
    header
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| DeliveryRow {
            incremento: i + 1,
            invoice: header.invoice.clone(),
            date: item.date.clone(),
            authorization: item.authorization.clone(),
            name: header.name.clone(),
            doc_type: header.doc_type.clone(),
            doc_number: header.doc_number.clone(),
            article: item.article.clone(),
            quantity: item.quantity.clone(),
        })
        .collect()
}

fn render_delivery(
    header: &Header,
    logo: std::result::Result<&[u8], &str>,
    signature: &[u8],
) -> Result<Vec<u8>> {
    let logo = logo.map_err(|reason| anyhow::anyhow!("{reason}"))?;
    delivery::render(&delivery_rows(header), logo, signature)
}

/// Renders the delivery page of the manual support with the patient signature.
fn manual_page(
    req: &Request<'_>,
    manual_signature: &dyn Fn(&Header) -> Result<ManualSignature>,
) -> std::result::Result<(Vec<u8>, ManualSignature), Built> {
    let signature = manual_signature(req.header)
        .map_err(|err| fail(CODE_RENDER_FAILED, format!("soporte manual: {err:#}")))?;
    let page = render_delivery(req.header, req.logo, &signature.png)
        .map_err(|err| fail(CODE_RENDER_FAILED, format!("{REPORT_DELIVERY}: {err:#}")))?;
    Ok((page, signature))
}
