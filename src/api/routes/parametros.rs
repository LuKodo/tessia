//! `GET`/`PUT /api/parameters`: the global preparation parameters (Go:
//! `cmd_serve_parameters.go`) over [`ParametrosStore`].
//!
//! Validation and its Spanish messages are the store's
//! ([`Parametros::validate`]); Go answered them in English. Store failures are
//! generic messages, never the driver text.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::api::{
    ApiError, ApiResult, ApiState,
    http::{MAX_JSON_BODY, read_json_exact},
    request::{Request, Response},
};
use crate::stores::parametros::{Parametros, ParametrosStore};
use crate::util::decimal::parse_max_amount;

pub const READ_FAILED: &str = "no se pudo leer los parámetros";
pub const WRITE_FAILED: &str = "no se pudo guardar los parámetros";

/// The answer of both methods.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ParametersResponse {
    /// False while nothing was saved and the defaults are shown.
    pub configured: bool,
    /// Pesos as text, without trailing zeros (`"1500000"`).
    pub max_invoice_amount: String,
    pub automatic_subplans: Vec<String>,
    pub wave_size: i16,
    pub derechos_unavailable_percent: i16,
    pub derechos_min_sample: i16,
    pub cuv_token_failure_percent: i16,
    pub cuv_min_sample: i16,
}

impl ParametersResponse {
    fn saved(params: &Parametros) -> Self {
        Self::new(
            true,
            params.max_invoice_amount.normalize().to_string(),
            params,
        )
    }

    fn new(configured: bool, amount: String, params: &Parametros) -> Self {
        Self {
            configured,
            max_invoice_amount: amount,
            automatic_subplans: params.automatic_subplans.clone(),
            wave_size: params.wave_size,
            derechos_unavailable_percent: params.derechos_unavailable_percent,
            derechos_min_sample: params.derechos_min_sample,
            cuv_token_failure_percent: params.cuv_token_failure_percent,
            cuv_min_sample: params.cuv_min_sample,
        }
    }
}

/// Request body of `PUT`. Absent or `null` fields take the zero value, as in Go,
/// so validation names what is missing.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParametersBody {
    #[serde(default)]
    max_invoice_amount: Option<String>,
    #[serde(default)]
    automatic_subplans: Option<Vec<String>>,
    #[serde(default)]
    wave_size: Option<i64>,
    #[serde(default)]
    derechos_unavailable_percent: Option<i64>,
    #[serde(default)]
    derechos_min_sample: Option<i64>,
    #[serde(default)]
    cuv_token_failure_percent: Option<i64>,
    #[serde(default)]
    cuv_min_sample: Option<i64>,
}

/// An out-of-range number saturates, so [`Parametros::validate`] rejects it
/// with its own message instead of the decoder failing on the width.
fn small(value: Option<i64>) -> i16 {
    let value = value.unwrap_or(0);
    i16::try_from(value).unwrap_or(if value < 0 { i16::MIN } else { i16::MAX })
}

impl ParametersBody {
    fn into_params(self) -> Result<Parametros, ApiError> {
        let text = self.max_invoice_amount.unwrap_or_default();
        let amount: Decimal =
            parse_max_amount(text.trim()).map_err(|err| ApiError::bad_request(err.to_string()))?;
        let params = Parametros {
            max_invoice_amount: amount,
            automatic_subplans: self.automatic_subplans.unwrap_or_default(),
            wave_size: small(self.wave_size),
            derechos_unavailable_percent: small(self.derechos_unavailable_percent),
            derechos_min_sample: small(self.derechos_min_sample),
            cuv_token_failure_percent: small(self.cuv_token_failure_percent),
            cuv_min_sample: small(self.cuv_min_sample),
        };
        params.validate().map_err(ApiError::bad_request)?;
        Ok(params)
    }
}

fn store(state: &ApiState) -> ParametrosStore {
    ParametrosStore::new(state.pool.clone())
}

pub fn get(state: &ApiState, _req: &Request) -> ApiResult {
    let stored = store(state).get().map_err(|err| {
        tracing::warn!(error = %err, "parameters: could not read");
        ApiError::internal(READ_FAILED)
    })?;
    let response = match stored {
        Some(params) => ParametersResponse::saved(&params),
        None => {
            // Nothing saved: the defaults, with the cap taken from the
            // configuration as written (possibly blank), like Go.
            let fallback = state
                .ctx
                .settings
                .get("convenio.max_invoice_amount")
                .unwrap_or("")
                .trim()
                .to_owned();
            ParametersResponse::new(false, fallback, &Parametros::defaults(Decimal::ZERO))
        }
    };
    Ok(Response::json(&response))
}

pub fn put(state: &ApiState, req: &Request) -> ApiResult {
    let params = read_json_exact::<ParametersBody>(req, MAX_JSON_BODY)?.into_params()?;
    store(state).set(&params).map_err(|err| {
        tracing::warn!(error = %err, "parameters: could not save");
        ApiError::internal(WRITE_FAILED)
    })?;
    Ok(Response::json(&ParametersResponse::saved(&params)))
}
