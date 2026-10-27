//! `parametros`: the single row (`id = 1`) of global preparation parameters
//! (Go `preparation_parameters`). Reading a never-saved row is not an error:
//! callers apply [`Parametros::defaults`].

mod schema;

use std::collections::HashSet;

use diesel::prelude::*;
use rust_decimal::Decimal;

use crate::db::{DbError, DbPool, DbResult};
use crate::util::text::normalize_key;
use schema::parametros as t;

/// Defaults of Go `convenio` (used when nothing was saved).
pub const DEFAULT_WAVE_SIZE: i16 = 20;
pub const MAX_WAVE_SIZE: i16 = 500;
pub const DEFAULT_DERECHOS_UNAVAILABLE_PERCENT: i16 = 50;
pub const DEFAULT_DERECHOS_MIN_SAMPLE: i16 = 4;
pub const MAX_DERECHOS_MIN_SAMPLE: i16 = 1000;
pub const DEFAULT_CUV_TOKEN_FAILURE_PERCENT: i16 = 50;
pub const DEFAULT_CUV_MIN_SAMPLE: i16 = 4;
pub const MAX_CUV_MIN_SAMPLE: i16 = 1000;

/// Subplans filed automatically unless the operator saves another list.
pub fn default_automatic_subplans() -> Vec<String> {
    [
        "EVENTO POS",
        "EVENTO NO PBS",
        "TRANSFERENCIA DE PENDIENTES CON AUTORIZACION",
        "EVENTO SAS SIN AUTORIZACION",
    ]
    .map(String::from)
    .to_vec()
}

/// The global automatic-preparation rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parametros {
    /// Per-invoice COP cap (`NUMERIC(14,2)`, positive, at most 12 integer digits).
    pub max_invoice_amount: Decimal,
    pub automatic_subplans: Vec<String>,
    /// Invoices a `proceso run` phase admits per wave; 0 disables the cap.
    pub wave_size: i16,
    /// Share (0-100) of affiliates that may end "validator unavailable"
    /// before `derechos validar` fails; 0 disables the alert.
    pub derechos_unavailable_percent: i16,
    /// Fewest queried affiliates for that alert to apply.
    pub derechos_min_sample: i16,
    /// Share (0-100) of submitted invoices that may fail the SISPRO login
    /// before the cuv phase fails; 0 disables the alert.
    pub cuv_token_failure_percent: i16,
    /// Fewest submitted invoices for that alert to apply.
    pub cuv_min_sample: i16,
}

impl Parametros {
    /// The values applied when nothing was saved, with `max_invoice_amount`
    /// supplied by the caller (typically `convenio.max_invoice_amount`).
    pub fn defaults(max_invoice_amount: Decimal) -> Self {
        Self {
            max_invoice_amount,
            automatic_subplans: default_automatic_subplans(),
            wave_size: DEFAULT_WAVE_SIZE,
            derechos_unavailable_percent: DEFAULT_DERECHOS_UNAVAILABLE_PERCENT,
            derechos_min_sample: DEFAULT_DERECHOS_MIN_SAMPLE,
            cuv_token_failure_percent: DEFAULT_CUV_TOKEN_FAILURE_PERCENT,
            cuv_min_sample: DEFAULT_CUV_MIN_SAMPLE,
        }
    }

    /// Rejects values that cannot safely be applied (Go `Validate`).
    pub fn validate(&self) -> Result<(), String> {
        let amount = self.max_invoice_amount;
        if amount <= Decimal::ZERO {
            return Err("el monto máximo debe ser positivo".into());
        }
        if amount.scale() > 2 || amount >= Decimal::from(1_000_000_000_000_i64) {
            return Err(
                "monto máximo inválido: use pesos sin separador de miles y máximo dos decimales"
                    .into(),
            );
        }
        if !(0..=MAX_WAVE_SIZE).contains(&self.wave_size) {
            return Err(format!(
                "el tamaño de oleada debe estar entre 0 (desactivado) y {MAX_WAVE_SIZE}"
            ));
        }
        if !(0..=100).contains(&self.derechos_unavailable_percent) {
            return Err(
                "el porcentaje de derechos no disponibles debe estar entre 0 (desactivado) y 100"
                    .into(),
            );
        }
        if !(0..=MAX_DERECHOS_MIN_SAMPLE).contains(&self.derechos_min_sample) {
            return Err(format!(
                "la muestra mínima de derechos debe estar entre 0 y {MAX_DERECHOS_MIN_SAMPLE}"
            ));
        }
        if !(0..=100).contains(&self.cuv_token_failure_percent) {
            return Err(
                "el porcentaje de fallos de token CUV debe estar entre 0 (desactivado) y 100"
                    .into(),
            );
        }
        if !(0..=MAX_CUV_MIN_SAMPLE).contains(&self.cuv_min_sample) {
            return Err(format!(
                "la muestra mínima de CUV debe estar entre 0 y {MAX_CUV_MIN_SAMPLE}"
            ));
        }
        if self.automatic_subplans.is_empty() {
            return Err("se requiere al menos un subplan automático".into());
        }
        let mut seen = HashSet::new();
        for subplan in &self.automatic_subplans {
            if subplan.trim().is_empty() {
                return Err("los subplanes automáticos no pueden estar en blanco".into());
            }
            if !seen.insert(normalize_key(subplan)) {
                return Err(format!("subplan automático duplicado «{}»", subplan.trim()));
            }
        }
        Ok(())
    }

    /// The allowlist as normalized keys (case/accent/spacing-insensitive match).
    pub fn automatic_subplan_keys(&self) -> HashSet<String> {
        self.automatic_subplans
            .iter()
            .map(|s| normalize_key(s))
            .collect()
    }
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = t)]
struct Row {
    max_invoice_amount: Decimal,
    automatic_subplans: serde_json::Value,
    wave_size: i16,
    derechos_unavailable_percent: i16,
    derechos_min_sample: i16,
    cuv_token_failure_percent: i16,
    cuv_min_sample: i16,
}

#[derive(Clone)]
pub struct ParametrosStore {
    pool: DbPool,
}

impl ParametrosStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// The saved parameters, `None` when never saved.
    pub fn get(&self) -> DbResult<Option<Parametros>> {
        let row: Option<Row> = self.pool.run("get parametros", |conn| {
            Ok(t::table
                .find(1_i16)
                .select(Row::as_select())
                .first(conn)
                .optional()?)
        })?;
        let Some(row) = row else { return Ok(None) };
        let subplans: Vec<String> =
            serde_json::from_value(row.automatic_subplans).map_err(|err| {
                DbError::other(format!(
                    "subplanes automáticos almacenados inválidos: {err}"
                ))
            })?;
        let params = Parametros {
            max_invoice_amount: row.max_invoice_amount,
            automatic_subplans: subplans,
            wave_size: row.wave_size,
            derechos_unavailable_percent: row.derechos_unavailable_percent,
            derechos_min_sample: row.derechos_min_sample,
            cuv_token_failure_percent: row.cuv_token_failure_percent,
            cuv_min_sample: row.cuv_min_sample,
        };
        params
            .validate()
            .map_err(|err| DbError::other(format!("parámetros almacenados inválidos: {err}")))?;
        Ok(Some(params))
    }

    /// The saved parameters, or [`Parametros::defaults`] with `fallback_amount`.
    pub fn get_or_default(&self, fallback_amount: Decimal) -> DbResult<Parametros> {
        Ok(self
            .get()?
            .unwrap_or_else(|| Parametros::defaults(fallback_amount)))
    }

    /// Validates and replaces the singleton row.
    pub fn set(&self, params: &Parametros) -> DbResult<()> {
        params.validate().map_err(DbError::Other)?;
        let subplans = serde_json::to_value(&params.automatic_subplans)
            .map_err(|err| DbError::other(format!("no se pudo codificar los subplanes: {err}")))?;
        let values = || {
            (
                t::id.eq(1_i16),
                t::max_invoice_amount.eq(params.max_invoice_amount),
                t::automatic_subplans.eq(subplans.clone()),
                t::wave_size.eq(params.wave_size),
                t::derechos_unavailable_percent.eq(params.derechos_unavailable_percent),
                t::derechos_min_sample.eq(params.derechos_min_sample),
                t::cuv_token_failure_percent.eq(params.cuv_token_failure_percent),
                t::cuv_min_sample.eq(params.cuv_min_sample),
            )
        };
        self.pool.run("set parametros", |conn| {
            diesel::insert_into(t::table)
                .values(values())
                .on_conflict(t::id)
                .do_update()
                .set((values(), t::updated_at.eq(diesel::dsl::now)))
                .execute(conn)?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use diesel::RunQueryDsl;

    use super::*;
    use crate::db::test_support::TestDb;

    fn amount(text: &str) -> Decimal {
        text.parse().expect("decimal")
    }

    #[test]
    fn defaults_apply_until_something_is_saved() {
        let db = TestDb::new();
        let store = ParametrosStore::new(db.pool());
        assert!(store.get().expect("get").is_none());
        let params = store.get_or_default(amount("100000")).expect("defaults");
        assert_eq!(params, Parametros::defaults(amount("100000")));
        assert_eq!(
            (
                params.wave_size,
                params.derechos_min_sample,
                params.cuv_token_failure_percent
            ),
            (20, 4, 50)
        );
        assert_eq!(params.automatic_subplans.len(), 4);
    }

    #[test]
    fn set_replaces_the_singleton() {
        let db = TestDb::new();
        let store = ParametrosStore::new(db.pool());
        let mut params = Parametros::defaults(amount("100000"));
        store.set(&params).expect("first");
        params.max_invoice_amount = amount("250000.50");
        params.automatic_subplans = vec!["Evento POS".into(), "Otro Plan".into()];
        params.wave_size = 0;
        store.set(&params).expect("second");
        assert_eq!(store.get().expect("get"), Some(params));
        let mut conn = db.conn();
        let second_row = diesel::sql_query(
            "INSERT INTO parametros (id, max_invoice_amount, automatic_subplans) VALUES (2, 1, '[]')",
        )
        .execute(&mut conn);
        assert!(second_row.is_err(), "the table holds a single row");
    }

    #[test]
    fn validation_follows_the_go_rules() {
        let base = Parametros::defaults(amount("100"));
        let bad = |mutate: &dyn Fn(&mut Parametros)| {
            let mut p = base.clone();
            mutate(&mut p);
            p.validate().is_err()
        };
        assert!(base.validate().is_ok());
        assert!(bad(&|p| p.max_invoice_amount = amount("0")));
        assert!(bad(&|p| p.max_invoice_amount = amount("1.234")));
        assert!(bad(&|p| p.wave_size = 501));
        assert!(bad(&|p| p.derechos_unavailable_percent = 101));
        assert!(bad(&|p| p.cuv_min_sample = 1001));
        assert!(bad(&|p| p.automatic_subplans.clear()));
        assert!(bad(&|p| p.automatic_subplans = vec![" ".into()]));
        assert!(bad(
            &|p| p.automatic_subplans = vec!["Evento pos".into(), "EVENTO  POS".into()]
        ));
    }

    #[test]
    fn invalid_values_are_not_written() {
        let db = TestDb::new();
        let store = ParametrosStore::new(db.pool());
        let mut params = Parametros::defaults(amount("100"));
        params.wave_size = 9999;
        assert!(store.set(&params).is_err());
        assert!(store.get().expect("get").is_none());
    }

    #[test]
    fn subplan_keys_ignore_case_accents_and_spacing() {
        let params = Parametros::defaults(amount("1"));
        assert!(
            params
                .automatic_subplan_keys()
                .contains(&normalize_key("evento  pos"))
        );
    }
}
