//! The totals compared before step 1 (Go `radicacion.ValueStore`): the XML
//! total lives in `factura_valor_xml`; the agreement total is supplied by the
//! caller (the dataset store, see [`Values::from_dataset`]).

use anyhow::Result;
use rust_decimal::Decimal;

use crate::stores::dataset::DatasetStore;
use crate::stores::valor_xml::ValorXmlStore;
use crate::util::decimal::{format2, parse_plain};

/// Supplies the agreement (convenio) total of an invoice; `None` = not found.
pub type AgreementTotal = Box<dyn Fn(&str) -> Result<Option<Decimal>> + Send + Sync>;

pub struct Values {
    xml: ValorXmlStore,
    agreement: AgreementTotal,
}

impl Values {
    pub fn new(xml: ValorXmlStore, agreement: AgreementTotal) -> Values {
        Values { xml, agreement }
    }

    /// The production wiring: the agreement total is derived from the newest
    /// current cargue that holds the invoice as `PREPARADA`.
    pub fn from_dataset(xml: ValorXmlStore, dataset: DatasetStore) -> Values {
        Values::new(
            xml,
            Box::new(move |invoice| Ok(dataset.agreement_total(invoice)?)),
        )
    }

    /// Stores the XML total read from the ZIP (replaces amount, source and path).
    pub fn sync_xml_total(&self, invoice: &str, amount: &str, source: &str, zip: &str) -> Result<()> {
        let amount = parse_plain(amount.trim())?;
        Ok(self.xml.sync_xml_total(invoice, amount, source, zip)?)
    }

    /// The stored XML total (the fallback when the ZIP cannot be read); a row
    /// without an amount counts as "not stored".
    pub fn stored_xml_total(&self, invoice: &str) -> Result<Option<(String, String)>> {
        Ok(self
            .xml
            .stored_xml_total(invoice)?
            .map(|(amount, source)| (format2(amount), source)))
    }

    /// The agreement total with two decimals.
    pub fn agreement_total(&self, invoice: &str) -> Result<Option<String>> {
        Ok((self.agreement)(invoice)?.map(format2))
    }
}
