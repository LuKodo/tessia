//! The SISPRO login-failure alert of a standalone `cuv generar` run
//! (Go `internal/cuv/availability.go`).

use std::sync::Mutex;

use crate::stores::parametros::{DEFAULT_CUV_MIN_SAMPLE, DEFAULT_CUV_TOKEN_FAILURE_PERCENT};

/// Counts the invoices of a cuv run that reached SISPRO.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenStats {
    /// Invoices for which `CargarFevRips` was attempted.
    pub submitted: usize,
    /// Of those, the ones that ended without a login token.
    pub token_failed: usize,
}

/// A thread-safe [`TokenStats`] shared by the workers of one process. Under
/// `proceso run` (one call per invoice) one tally spans the whole run: only the
/// whole run is a meaningful sample.
#[derive(Debug, Default)]
pub struct TokenTally {
    stats: Mutex<TokenStats>,
}

impl TokenTally {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, token_failed: bool) {
        let mut stats = self
            .stats
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        stats.submitted += 1;
        if token_failed {
            stats.token_failed += 1;
        }
    }

    pub fn snapshot(&self) -> TokenStats {
        *self
            .stats
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// When a share of login failures means SISPRO (or its credentials) is down
/// rather than a few invoices being unlucky.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenPolicy {
    /// Share (0-100) of submitted invoices that may fail the login before the
    /// run is an error; 0 disables the check.
    pub percent: i64,
    /// Fewest submitted invoices for the check to apply; 0 = no minimum.
    pub min_sample: i64,
}

impl Default for TokenPolicy {
    fn default() -> Self {
        Self {
            percent: i64::from(DEFAULT_CUV_TOKEN_FAILURE_PERCENT),
            min_sample: i64::from(DEFAULT_CUV_MIN_SAMPLE),
        }
    }
}

/// Login failed for more than the allowed share of the submitted invoices.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "SISPRO: {token_failed} de {submitted} facturas enviadas fallaron al iniciar sesion sin token ({share}%), por encima del umbral de {percent}%; las facturas quedaron en FAILED y se reintentan en la siguiente ejecucion cuando SISPRO responda"
)]
pub struct TokenFailureError {
    pub token_failed: usize,
    pub submitted: usize,
    pub percent: i64,
    /// `token_failed * 100 / submitted`, truncated.
    pub share: usize,
}

/// `CheckTokenFailures`: an error when the login failures of `stats` exceed
/// `policy.percent` of the submitted invoices and at least `min_sample` were
/// submitted.
pub fn check_token_failures(
    stats: TokenStats,
    policy: TokenPolicy,
) -> Result<(), TokenFailureError> {
    let submitted = stats.submitted as i64;
    if policy.percent <= 0 || submitted <= 0 || submitted < policy.min_sample {
        return Ok(());
    }
    if (stats.token_failed as i64) * 100 <= policy.percent * submitted {
        return Ok(());
    }
    Err(TokenFailureError {
        token_failed: stats.token_failed,
        submitted: stats.submitted,
        percent: policy.percent,
        share: stats.token_failed * 100 / stats.submitted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(submitted: usize, token_failed: usize) -> TokenStats {
        TokenStats {
            submitted,
            token_failed,
        }
    }

    #[test]
    fn the_default_policy_is_fifty_percent_from_four_invoices() {
        let policy = TokenPolicy::default();
        assert_eq!((policy.percent, policy.min_sample), (50, 4));
    }

    #[test]
    fn only_a_share_strictly_above_the_threshold_trips_the_alert() {
        let policy = TokenPolicy::default();
        assert!(
            check_token_failures(stats(4, 2), policy).is_ok(),
            "exactly 50%"
        );
        let err = check_token_failures(stats(4, 3), policy).expect_err("75%");
        assert_eq!(
            err.to_string(),
            "SISPRO: 3 de 4 facturas enviadas fallaron al iniciar sesion sin token (75%), por encima del umbral de 50%; las facturas quedaron en FAILED y se reintentan en la siguiente ejecucion cuando SISPRO responda"
        );
        assert!(
            check_token_failures(stats(3, 3), policy).is_ok(),
            "below the minimum sample"
        );
        assert!(check_token_failures(stats(0, 0), policy).is_ok());
    }

    #[test]
    fn zero_percent_disables_and_zero_minimum_applies_to_one_invoice() {
        let off = TokenPolicy {
            percent: 0,
            min_sample: 0,
        };
        assert!(check_token_failures(stats(10, 10), off).is_ok());
        let any = TokenPolicy {
            percent: 50,
            min_sample: 0,
        };
        assert!(check_token_failures(stats(1, 1), any).is_err());
    }

    #[test]
    fn the_tally_counts_submissions_and_token_failures() {
        let tally = TokenTally::new();
        tally.record(false);
        tally.record(true);
        tally.record(true);
        assert_eq!(tally.snapshot(), stats(3, 2));
    }
}
