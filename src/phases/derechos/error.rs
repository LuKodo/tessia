//! Typed failures of `derechos validar`. They travel inside `anyhow::Error`, so
//! `proceso run` can `downcast_ref::<DerechosError>()` to tell an
//! authentication stop (fatal for the whole pipeline) from the others.

use super::rights::REASON_UNAVAILABLE;

/// Code under which `convenio preparar` holds invoices of several affiliates.
const CODE_MULTIPLE_AFFILIATES: &str = "MULTIPLES_AFILIADOS_EN_FACTURA";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DerechosError {
    /// An invoice names several affiliates: a dataset that breaks the
    /// exactly-one-affiliate rule. The message names the invoice, never the
    /// documents (personal data).
    #[error(
        "derechos: invoice {invoice} has lines of more than one affiliate; convenio preparar must hold it as {CODE_MULTIPLE_AFFILIATES} (re-run convenio preparar)"
    )]
    MultipleAffiliates { invoice: String },
    /// The validator rejected the token even after renewing it (or no token
    /// could be obtained). Resolved invoices were saved; `untouched` were
    /// restored to their previous state.
    #[error(
        "validador de derechos: credenciales o token rechazados aun despues de renovarlo; {untouched} factura(s) quedaron sin tocar, corrige las credenciales y ejecuta de nuevo: derechos: validator authentication failed: {detail}"
    )]
    AuthenticationStopped { untouched: usize, detail: String },
    /// Too many technical failures: the validator looks down. Outcomes were
    /// saved first; the invoices stay in `REVISION_MANUAL`.
    #[error(
        "{REASON_UNAVAILABLE}: {unavailable} de {queried} afiliados consultados fallaron tecnicamente ({share}%), por encima del umbral de {percent}%; las facturas quedaron en REVISION_MANUAL y solo se reintentan con --invoices cuando el validador responda",
        share = .unavailable * 100 / .queried
    )]
    ValidatorUnavailable {
        unavailable: usize,
        queried: usize,
        percent: u32,
    },
    /// Ctrl-C: nothing was saved, the claims were released.
    #[error("derechos: interrumpido; las facturas sin resolver volvieron a su estado anterior")]
    Interrupted,
}
