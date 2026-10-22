//! Result codes and the branch decision per invoice (Go `soporte.Decide`),
//! made only from the stored CPScan lookup.

use super::cpscan::Stored;
use super::input::{Header, is_deposit, is_mandatory_deposit, is_sede_002};
use crate::domain::PhaseState;

pub const CODE_OK: &str = "OK";
pub const CODE_EXISTS: &str = "YA_EXISTE";
pub const CODE_NO_SSC: &str = "SIN_SSC";
pub const CODE_NO_INVOICE_PDF: &str = "FACTURA_PDF_FALTA";
/// The invoice number is not a valid `radicacion` invoice, so no key is ever derived from it.
pub const CODE_INVALID_INVOICE: &str = "FACTURA_INVALIDA";
pub const CODE_MANDATORY_MISSING: &str = "SOPORTE_OBLIGATORIO_NO_DISPONIBLE";
pub const CODE_PC_WITHOUT_URL: &str = "PC_SIN_URL";
pub const CODE_CPSCAN_MISSING: &str = "CPSCAN_NO_DESCARGADO";
pub const CODE_DEPOSIT_FORMAT_MISSING: &str = "FORMATO_DEPOSITO_NO_DISPONIBLE";
pub const CODE_RENDER_FAILED: &str = "RENDER_FALLIDO";
pub const CODE_MERGE_FAILED: &str = "UNION_PDF_FALLIDA";

/// Name of the report that has no Rust renderer (the Jasper sidecar is not ported).
pub const REPORT_DELIVERY_DEPOSIT: &str = "Formato_App_Deposito";

/// Proceso state of a result code: `DONE` for `OK`/`YA_EXISTE`;
/// `REVISION_MANUAL` for data a person must fix (`SIN_SSC`, `PC_SIN_URL`);
/// `FAILED` (retried) for everything that may resolve by itself or upstream.
pub fn state_of(code: &str) -> PhaseState {
    match code {
        CODE_OK | CODE_EXISTS => PhaseState::Done,
        CODE_NO_SSC | CODE_PC_WITHOUT_URL => PhaseState::RevisionManual,
        _ => PhaseState::Failed,
    }
}

/// The way the support is assembled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Branch {
    /// CPScan formula image + [delivery page] + invoice (origen APP, BUCKET or empty).
    App,
    /// Like APP, the image is the `firma` image when CPScan has one.
    Sl,
    /// CPScan scanned PDF used as is + invoice.
    Pc,
    /// No usable document (or unknown origen): formula image + invoice.
    Formula,
    /// CPScan has no support: delivery page with the patient signature + invoice.
    Manual,
}

impl Branch {
    pub fn as_str(self) -> &'static str {
        match self {
            Branch::App => "APP",
            Branch::Sl => "SL",
            Branch::Pc => "PC",
            Branch::Formula => "FORMULA",
            Branch::Manual => "MANUAL",
        }
    }
}

/// The formula page template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormulaReport {
    /// `Formato_Formula`.
    Standard,
    /// `Formato_Formula_Deposito_1`.
    Deposit1,
}

impl FormulaReport {
    pub fn name(self) -> &'static str {
        match self {
            FormulaReport::Standard => "Formato_Formula",
            FormulaReport::Deposit1 => "Formato_Formula_Deposito_1",
        }
    }
}

/// The plan for one invoice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// It cannot be built.
    Fail { code: &'static str, detail: String },
    Build {
        branch: Branch,
        formula: Option<FormulaReport>,
        /// A delivery page (`Formato_App`) is added when a signature is available.
        delivery: bool,
        deposit: bool,
        /// Marks the manual support.
        detail: String,
    },
}

fn fail(code: &'static str, detail: impl Into<String>) -> Decision {
    Decision::Fail {
        code,
        detail: detail.into(),
    }
}

/// Go `Decide`; `stored` is the sidecar of the invoice (`None`: never fetched).
pub fn decide(header: &Header, stored: Option<&Stored>) -> Decision {
    if header.ssc.is_empty() {
        return fail(CODE_NO_SSC, "");
    }
    let Some(stored) = stored else {
        return fail(CODE_CPSCAN_MISSING, "");
    };
    let ceco = header.cost_centre.as_str();
    let (deposit, mandatory) = (is_deposit(ceco), is_mandatory_deposit(ceco));
    let doc = stored.document();
    if !stored.has_file() {
        return decide_without_file(header, doc.as_ref(), deposit, mandatory);
    }
    if stored.ext == super::cpscan::EXT_PDF {
        return Decision::Build {
            branch: Branch::Pc,
            formula: None,
            delivery: false,
            deposit,
            detail: String::new(),
        };
    }
    let formula = if deposit {
        FormulaReport::Deposit1
    } else {
        FormulaReport::Standard
    };
    let (mut branch, mut known) = (Branch::App, false);
    if let Some(doc) = &doc {
        match doc.normalized_origin().as_str() {
            "APP" | "NULL" | "BUCKET" => known = true,
            "SL" => (branch, known) = (Branch::Sl, true),
            _ => known = stored.from_additional(),
        }
    }
    if known {
        // Mandatory deposits call generar_VistaUnificada without a signature.
        return Decision::Build {
            branch,
            formula: Some(formula),
            delivery: !mandatory,
            deposit,
            detail: String::new(),
        };
    }
    if mandatory {
        return fail(
            CODE_MANDATORY_MISSING,
            format!("CPScan sin documento valido para {ceco}"),
        );
    }
    // generar_Deposito_Formula always uses Formato_Formula_Deposito_1; only the
    // deposit call passes the signature path.
    Decision::Build {
        branch: Branch::Formula,
        formula: Some(FormulaReport::Deposit1),
        delivery: deposit,
        deposit,
        detail: String::new(),
    }
}

fn decide_without_file(
    header: &Header,
    doc: Option<&super::cpscan::Document>,
    deposit: bool,
    mandatory: bool,
) -> Decision {
    const NO_SUPPORT: &str = "CPScan sin soporte";
    let ceco = header.cost_centre.as_str();
    let pc_without_url =
        doc.is_some_and(|d| d.normalized_origin() == "PC" && d.additional.trim().is_empty());
    if pc_without_url {
        if mandatory {
            return fail(CODE_MANDATORY_MISSING, format!("{NO_SUPPORT}: PC sin URL"));
        }
        return fail(CODE_PC_WITHOUT_URL, "");
    }
    if mandatory {
        return fail(CODE_MANDATORY_MISSING, format!("{NO_SUPPORT} para {ceco}"));
    }
    if is_sede_002(ceco) {
        return fail(CODE_MANDATORY_MISSING, format!("{NO_SUPPORT} en sede 002"));
    }
    if deposit {
        return fail(
            CODE_DEPOSIT_FORMAT_MISSING,
            format!(
                "{NO_SUPPORT}: el soporte manual de un deposito necesita {REPORT_DELIVERY_DEPOSIT} (pending-inputs #1)"
            ),
        );
    }
    Decision::Build {
        branch: Branch::Manual,
        formula: None,
        delivery: false,
        deposit: false,
        detail: format!("soporte manual: {NO_SUPPORT}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(ceco: &str) -> Header {
        Header {
            invoice: "FEV1".into(),
            ssc: "10".into(),
            cost_centre: ceco.into(),
            ..Header::default()
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

    fn nothing(origin: &str, additional: &str) -> Stored {
        stored(origin, additional, "", "")
    }

    fn code(d: &Decision) -> &'static str {
        match d {
            Decision::Fail { code, .. } => code,
            Decision::Build { .. } => "BUILD",
        }
    }

    fn branch(d: &Decision) -> (Branch, bool, Option<FormulaReport>) {
        match d {
            Decision::Build {
                branch,
                delivery,
                formula,
                ..
            } => (*branch, *delivery, *formula),
            Decision::Fail { code, .. } => panic!("unexpected {code}"),
        }
    }

    #[test]
    fn no_ssc_and_no_sidecar_are_reported_first() {
        let mut h = header("CENTRO");
        h.ssc.clear();
        assert_eq!(code(&decide(&h, None)), CODE_NO_SSC);
        assert_eq!(code(&decide(&header("CENTRO"), None)), CODE_CPSCAN_MISSING);
    }

    #[test]
    fn a_pdf_is_the_pc_branch_whatever_the_origin() {
        let d = decide(
            &header("CENTRO"),
            Some(&stored("PC", "http://x/a.pdf", "http://x/a.pdf", "pdf")),
        );
        assert_eq!(branch(&d), (Branch::Pc, false, None));
    }

    #[test]
    fn app_bucket_empty_and_null_origins_use_the_standard_formula_with_a_delivery_page() {
        for origin in ["APP", "BUCKET", "", "NULL", "app"] {
            let d = decide(
                &header("CENTRO"),
                Some(&stored(origin, "", "formula-10.jpg", "jpg")),
            );
            assert_eq!(
                branch(&d),
                (Branch::App, true, Some(FormulaReport::Standard)),
                "{origin}"
            );
        }
    }

    #[test]
    fn sl_is_like_app_with_its_own_branch_name() {
        let d = decide(
            &header("CENTRO"),
            Some(&stored("SL", "", "firma-10.png", "png")),
        );
        assert_eq!(
            branch(&d),
            (Branch::Sl, true, Some(FormulaReport::Standard))
        );
    }

    #[test]
    fn deposits_use_the_deposit_formula() {
        let d = decide(
            &header("BLOC PORT"),
            Some(&stored("APP", "", "formula-10.jpg", "jpg")),
        );
        assert_eq!(
            branch(&d),
            (Branch::App, true, Some(FormulaReport::Deposit1))
        );
        let d = decide(
            &header("BODEGA 1"),
            Some(&stored("APP", "", "formula-10.jpg", "jpg")),
        );
        assert_eq!(
            branch(&d),
            (Branch::App, false, Some(FormulaReport::Deposit1)),
            "mandatory deposits never get the page"
        );
    }

    #[test]
    fn an_unknown_origin_falls_back_to_formula_unless_the_image_came_from_adicional() {
        let d = decide(
            &header("CENTRO"),
            Some(&stored("XYZ", "", "formula-10.jpg", "jpg")),
        );
        assert_eq!(
            branch(&d),
            (Branch::Formula, false, Some(FormulaReport::Deposit1))
        );
        let d = decide(
            &header("DEPOSITO X"),
            Some(&stored("XYZ", "", "formula-10.jpg", "jpg")),
        );
        assert_eq!(
            branch(&d),
            (Branch::Formula, true, Some(FormulaReport::Deposit1))
        );
        let d = decide(
            &header("CENTRO"),
            Some(&stored("XYZ", "http://x/i.jpg", "http://x/i.jpg", "jpg")),
        );
        assert_eq!(
            branch(&d),
            (Branch::App, true, Some(FormulaReport::Standard))
        );
        let mut no_doc = stored("", "", "formula-10.jpg", "jpg");
        no_doc.found = false;
        assert_eq!(
            branch(&decide(&header("CENTRO"), Some(&no_doc))).0,
            Branch::Formula
        );
        let d = decide(
            &header("BODEGA 1"),
            Some(&stored("XYZ", "", "formula-10.jpg", "jpg")),
        );
        assert_eq!(code(&d), CODE_MANDATORY_MISSING);
    }

    #[test]
    fn cpscan_without_support_builds_the_manual_branch_for_plain_cost_centres() {
        let d = decide(&header("CENTRO"), Some(&nothing("APP", "")));
        let Decision::Build { branch, detail, .. } = d else {
            panic!("expected a build");
        };
        assert_eq!(branch, Branch::Manual);
        assert_eq!(detail, "soporte manual: CPScan sin soporte");
    }

    #[test]
    fn mandatory_deposits_and_sede_002_cannot_use_the_manual_support() {
        for ceco in [
            "BODEGA 1",
            "DEPOSITO SINCELEJO",
            "DEPOSITO CARTAGENA BLOC PORT",
            "SEDE 002",
            "002",
        ] {
            let d = decide(&header(ceco), Some(&nothing("APP", "")));
            assert_eq!(code(&d), CODE_MANDATORY_MISSING, "{ceco}");
        }
    }

    #[test]
    fn a_non_mandatory_deposit_needs_the_missing_deposit_format() {
        let d = decide(&header("BLOC PORT"), Some(&nothing("APP", "")));
        let Decision::Fail { code, detail } = d else {
            panic!("expected a failure");
        };
        assert_eq!(code, CODE_DEPOSIT_FORMAT_MISSING);
        assert!(detail.contains("Formato_App_Deposito"));
    }

    #[test]
    fn pc_without_an_address_needs_a_person_unless_it_is_mandatory() {
        assert_eq!(
            code(&decide(&header("CENTRO"), Some(&nothing("PC", "")))),
            CODE_PC_WITHOUT_URL
        );
        assert_eq!(
            code(&decide(&header("BODEGA 1"), Some(&nothing("PC", "  ")))),
            CODE_MANDATORY_MISSING
        );
    }

    #[test]
    fn states_follow_the_go_table() {
        assert_eq!(state_of(CODE_OK), PhaseState::Done);
        assert_eq!(state_of(CODE_EXISTS), PhaseState::Done);
        assert_eq!(state_of(CODE_NO_SSC), PhaseState::RevisionManual);
        assert_eq!(state_of(CODE_PC_WITHOUT_URL), PhaseState::RevisionManual);
        for failed in [
            CODE_NO_INVOICE_PDF,
            CODE_CPSCAN_MISSING,
            CODE_MANDATORY_MISSING,
            CODE_DEPOSIT_FORMAT_MISSING,
            CODE_RENDER_FAILED,
            CODE_MERGE_FAILED,
            CODE_INVALID_INVOICE,
        ] {
            assert_eq!(state_of(failed), PhaseState::Failed, "{failed}");
        }
    }
}
