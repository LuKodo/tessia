//! Where the invoice XML lives in the cargue's artifacts, and the XML totals kept
//! in `factura_valor_xml`.
//!
//! The Go tool looked at `<F>_XML_PDF.zip` under `XML/` and then `ZIP/`, and
//! at a loose `<F>.xml`, inside the load folder. Here the same names are looked
//! up in the `artefactos` index of the cargue and read straight from the bucket.

use std::collections::HashMap;

use anyhow::Result;
use rust_decimal::Decimal;

use crate::context::AppContext;
use crate::fevxml;
use crate::storage::Artifacts;
use crate::stores::artefactos::Artefacto;
use crate::stores::valor_xml::XmlValueUpsert;

/// Index rows of one cargue, by relative path.
pub struct XmlSources {
    cargue_id: String,
    artifacts: Artifacts,
    rows: HashMap<String, Artefacto>,
}

/// `(modified in unix milliseconds, size)` of the artifact a value was read
/// from: the stamp `factura_valor_xml` keeps next to the value.
type Stamp = (i64, i64);

impl XmlSources {
    pub fn load(artifacts: &Artifacts, cargue_id: &str) -> Result<Self> {
        let rows = artifacts
            .list(cargue_id)?
            .into_iter()
            .map(|row| (row.rel_path.clone(), row))
            .collect();
        Ok(Self {
            cargue_id: cargue_id.to_owned(),
            artifacts: artifacts.clone(),
            rows,
        })
    }

    fn non_empty(&self, rel: &str) -> Option<&Artefacto> {
        self.rows.get(rel).filter(|row| row.size > 0)
    }

    /// First non-empty `<F>_XML_PDF.zip` (`XML/` before `ZIP/`).
    fn xml_pdf_zip(&self, name: &str) -> Option<(String, &Artefacto)> {
        ["XML", "ZIP"].iter().find_map(|dir| {
            let rel = format!("{dir}/{name}_XML_PDF.zip");
            self.non_empty(&rel).map(|row| (rel, row))
        })
    }

    /// Reads the XML of invoice file name `name` once and serves both readers:
    /// the value check (Go `invoiceXML`: the first `.xml` entry of
    /// `<F>_XML_PDF.zip`, else the loose `<F>.xml`; a ZIP that cannot be read or
    /// has no XML entry falls back to the loose file) and the stored value (Go
    /// `LocateInvoiceXML` + `Total`: the ZIP when it exists, else the loose file).
    pub fn read(&self, name: &str) -> XmlRead {
        let zip = self.xml_pdf_zip(name);
        let zip_xml = zip.as_ref().and_then(|(rel, _)| {
            let bytes = self.artifacts.fetch_bytes(&self.cargue_id, rel).ok()?;
            fevxml::first_xml_entry(&bytes).ok().flatten()
        });
        let loose_rel = format!("{name}.xml");
        let loose_row = self.non_empty(&loose_rel);
        let loose = match (&zip_xml, loose_row) {
            (Some(_), _) | (None, None) => None,
            (None, Some(_)) => self.artifacts.fetch_bytes(&self.cargue_id, &loose_rel).ok(),
        };
        let tracked = match (zip, loose_row) {
            (Some((_, row)), _) => Some((
                stamp_of(row),
                zip_xml.as_deref().and_then(fevxml::read_total),
            )),
            (None, Some(row)) => {
                Some((stamp_of(row), loose.as_deref().and_then(fevxml::read_total)))
            }
            (None, None) => None,
        };
        XmlRead {
            verify_xml: zip_xml.or(loose.filter(|bytes| !bytes.is_empty())),
            tracked,
        }
    }
}

fn stamp_of(row: &Artefacto) -> Stamp {
    (row.created_at.timestamp_millis(), row.size)
}

/// What [`XmlSources::read`] found for an invoice.
pub struct XmlRead {
    /// The XML the value check compares (`None`: no XML at all).
    pub verify_xml: Option<Vec<u8>>,
    /// The file the stored value comes from and its total (`None` as a whole:
    /// the invoice has no XML file; `None` total: the file has no readable total).
    pub tracked: Option<(Stamp, Option<Decimal>)>,
}

impl XmlRead {
    /// The `factura_valor_xml` row of `name`, if the invoice has an XML file.
    pub fn upsert(&self, name: &str) -> Option<XmlValueUpsert> {
        let (stamp, total) = self.tracked.as_ref()?;
        Some(XmlValueUpsert {
            invoice: name.to_owned(),
            amount: total.and_then(|t| fevxml::storage_amount(t).parse().ok()),
            mtime_ms: stamp.0,
            size: stamp.1,
        })
    }
}

/// Go `persistXMLValues`: stores the XML total of each invoice (file name
/// without `_XML_PDF`) in `factura_valor_xml`. A side effect of the phase,
/// never part of its outcome: a failure is only logged.
pub fn persist_xml_values(ctx: &AppContext, cargue_id: &str, names: &[String]) {
    if names.is_empty() {
        return;
    }
    let sources = match XmlSources::load(&ctx.artifacts(), cargue_id) {
        Ok(sources) => sources,
        Err(err) => {
            tracing::warn!(error = %err, "xml total not stored");
            return;
        }
    };
    let values: Vec<XmlValueUpsert> = names
        .iter()
        .filter_map(|name| sources.read(name).upsert(name))
        .collect();
    if let Err(err) = ctx.valor_xml().upsert_xml_values(&values) {
        tracing::warn!(error = %err, "xml total not stored");
    }
}
