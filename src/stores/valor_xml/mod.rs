//! `factura_valor_xml`: the XML total read from an invoice ZIP, kept so the
//! value check still has it when the ZIP is gone.
//!
//! Two independent writers share the row (keyed by the normalized invoice,
//! see [`crate::domain::normalize_invoice`]):
//!
//! * the radicación gate: [`ValorXmlStore::sync_xml_total`] /
//!   [`ValorXmlStore::stored_xml_total`] (`valor_total_xml`, `fuente`,
//!   `ruta_zip`, `sincronizado_at`);
//! * the tracking view: [`ValorXmlStore::upsert_xml_values`] /
//!   [`ValorXmlStore::stored_xml_values`] (`xml_valor`, `xml_mtime`,
//!   `xml_size`), which only touch their own columns.
//!
//! The *agreement* total is never stored: it is derived from the prepared
//! dataset tables (owned by `convenio preparar`).

mod schema;

use std::collections::HashMap;

use diesel::prelude::*;
use rust_decimal::Decimal;

use super::chunks;
use crate::db::{DbPool, DbResult};
use crate::domain::normalize_invoice;
use schema::factura_valor_xml as t;

/// Tracking value of an invoice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlValue {
    /// `None`: a file that was read and has no readable total.
    pub amount: Option<Decimal>,
    /// Stat of the file the value came from (`mtime` in unix milliseconds,
    /// `size` in bytes); `None` for a row only the gate wrote.
    pub stamp: Option<(i64, i64)>,
}

impl XmlValue {
    /// Whether the file must be read again, from its current stat (`None` when
    /// the file is gone). A row without a stamp always needs it; a vanished
    /// file keeps the last value read.
    pub fn needs_refresh(&self, current: Option<(i64, i64)>) -> bool {
        match (current, self.stamp) {
            (None, _) => false,
            (Some(_), None) => true,
            (Some(now), Some(stored)) => now != stored,
        }
    }
}

/// One tracking upsert.
#[derive(Debug, Clone)]
pub struct XmlValueUpsert {
    pub invoice: String,
    pub amount: Option<Decimal>,
    pub mtime_ms: i64,
    pub size: i64,
}

/// `(num_factura, xml_valor, xml_mtime, xml_size)`.
type TrackingRow = (String, Option<Decimal>, Option<i64>, Option<i64>);

#[derive(Clone)]
pub struct ValorXmlStore {
    pool: DbPool,
}

impl ValorXmlStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Replaces the amount, source and ZIP path the gate keeps and stamps the
    /// sync time (one upsert per invoice).
    pub fn sync_xml_total(
        &self,
        invoice: &str,
        amount: Decimal,
        source: &str,
        zip_path: &str,
    ) -> DbResult<()> {
        let key = normalize_invoice(invoice);
        self.pool.run("sync xml total", |conn| {
            diesel::insert_into(t::table)
                .values((
                    t::num_factura.eq(&key),
                    t::valor_total_xml.eq(Some(amount)),
                    t::fuente.eq(Some(source)),
                    t::sincronizado_at.eq(diesel::dsl::now),
                    t::ruta_zip.eq(Some(zip_path)),
                ))
                .on_conflict(t::num_factura)
                .do_update()
                .set((
                    t::valor_total_xml.eq(Some(amount)),
                    t::fuente.eq(Some(source)),
                    t::sincronizado_at.eq(diesel::dsl::now),
                    t::ruta_zip.eq(Some(zip_path)),
                ))
                .execute(conn)?;
            Ok(())
        })
    }

    /// `(amount, source)` kept by the gate. A missing row and a row without an
    /// amount are both "not stored" (`None`).
    pub fn stored_xml_total(&self, invoice: &str) -> DbResult<Option<(Decimal, String)>> {
        let key = normalize_invoice(invoice);
        self.pool.run("stored xml total", |conn| {
            let row: Option<(Option<Decimal>, Option<String>)> = t::table
                .find(&key)
                .select((t::valor_total_xml, t::fuente))
                .first(conn)
                .optional()?;
            Ok(row.and_then(|(amount, source)| amount.map(|a| (a, source.unwrap_or_default()))))
        })
    }

    /// Upserts tracking values, creating the row or replacing only the
    /// tracking columns. Rows are written in invoice order so concurrent
    /// refreshes of overlapping pages take their locks in the same order.
    pub fn upsert_xml_values(&self, values: &[XmlValueUpsert]) -> DbResult<()> {
        let mut rows: Vec<(String, &XmlValueUpsert)> = values
            .iter()
            .map(|v| (normalize_invoice(&v.invoice), v))
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        for part in chunks(&rows) {
            self.pool.transaction("upsert xml values", |conn| {
                for (key, v) in part {
                    let tracking = (
                        t::xml_valor.eq(v.amount),
                        t::xml_mtime.eq(Some(v.mtime_ms)),
                        t::xml_size.eq(Some(v.size)),
                    );
                    diesel::insert_into(t::table)
                        .values((t::num_factura.eq(key), tracking))
                        .on_conflict(t::num_factura)
                        .do_update()
                        .set(tracking)
                        .execute(conn)?;
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    /// Tracking values keyed by normalized invoice; invoices without a row are
    /// absent.
    pub fn stored_xml_values(&self, invoices: &[String]) -> DbResult<HashMap<String, XmlValue>> {
        let mut keys: Vec<String> = invoices.iter().map(|i| normalize_invoice(i)).collect();
        keys.sort();
        keys.dedup();
        let mut out = HashMap::new();
        for part in chunks(&keys) {
            let rows: Vec<TrackingRow> = self.pool.run("stored xml values", |conn| {
                Ok(t::table
                    .filter(t::num_factura.eq_any(part))
                    .select((t::num_factura, t::xml_valor, t::xml_mtime, t::xml_size))
                    .load(conn)?)
            })?;
            for (key, amount, mtime, size) in rows {
                let stamp = mtime.zip(size);
                out.insert(key, XmlValue { amount, stamp });
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::TestDb;

    fn dec(text: &str) -> Decimal {
        text.parse().expect("decimal")
    }

    #[test]
    fn gate_total_is_keyed_by_the_normalized_invoice() {
        let db = TestDb::new();
        let store = ValorXmlStore::new(db.pool());
        assert_eq!(store.stored_xml_total("evs-1").expect("none"), None);
        store
            .sync_xml_total("evs-1", dec("1500.50"), "PayableAmount", "/w/EVS1.zip")
            .expect("sync");
        assert_eq!(
            store.stored_xml_total("EVS1").expect("get"),
            Some((dec("1500.50"), "PayableAmount".into()))
        );
        store
            .sync_xml_total("EVS1", dec("20"), "TaxInclusiveAmount", "/w/EVS1.zip")
            .expect("resync");
        assert_eq!(
            store.stored_xml_total("evs1").expect("get").map(|t| t.0),
            Some(dec("20.00"))
        );
    }

    #[test]
    fn tracking_and_gate_columns_do_not_overwrite_each_other() {
        let db = TestDb::new();
        let store = ValorXmlStore::new(db.pool());
        store
            .sync_xml_total("EVS1", dec("100"), "PayableAmount", "z")
            .expect("gate");
        let up = XmlValueUpsert {
            invoice: "evs-1".into(),
            amount: Some(dec("99.99")),
            mtime_ms: 1000,
            size: 10,
        };
        store.upsert_xml_values(&[up]).expect("tracking");
        assert_eq!(
            store.stored_xml_total("EVS1").expect("gate").map(|t| t.0),
            Some(dec("100.00"))
        );
        let values = store
            .stored_xml_values(&["EVS1".into(), "NOPE".into()])
            .expect("values");
        assert_eq!(values.len(), 1);
        assert_eq!(
            values["EVS1"],
            XmlValue {
                amount: Some(dec("99.99")),
                stamp: Some((1000, 10))
            }
        );
    }

    #[test]
    fn a_row_only_the_gate_wrote_has_no_stamp_and_needs_refresh() {
        let db = TestDb::new();
        let store = ValorXmlStore::new(db.pool());
        store
            .sync_xml_total("EVS2", dec("1"), "x", "z")
            .expect("gate");
        store
            .upsert_xml_values(&[XmlValueUpsert {
                invoice: "EVS3".into(),
                amount: None,
                mtime_ms: 5,
                size: 6,
            }])
            .expect("no amount");
        let values = store
            .stored_xml_values(&["EVS2".into(), "EVS3".into()])
            .expect("values");
        assert!(values["EVS2"].needs_refresh(Some((1, 1))));
        assert!(
            !values["EVS2"].needs_refresh(None),
            "a vanished file keeps what was read"
        );
        assert_eq!(values["EVS3"].amount, None);
        assert!(!values["EVS3"].needs_refresh(Some((5, 6))));
        assert!(values["EVS3"].needs_refresh(Some((5, 7))));
    }
}
