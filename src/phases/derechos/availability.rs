//! Validator-down alert (Go `internal/derechos/availability.go`).

use super::error::DerechosError;

/// Counts of the distinct affiliates of one validation run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// Distinct affiliates with a document.
    pub affiliates: usize,
    /// Answered from the rights cache (the validator was not asked).
    pub cached: usize,
    /// `affiliates - cached`: the validator was asked.
    pub queried: usize,
    /// Of `queried`, those that ended as a technical validator failure.
    pub unavailable: usize,
}

impl Stats {
    /// Accumulates `other` (several per-invoice runs of one process).
    pub fn add(&mut self, other: Stats) {
        self.affiliates += other.affiliates;
        self.cached += other.cached;
        self.queried += other.queried;
        self.unavailable += other.unavailable;
    }
}

/// When a share of technical failures means the validator is down rather
/// than a few affiliates being unlucky.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Share (0-100) of queried affiliates that may fail before the run is an
    /// error; 0 disables the check.
    pub percent: u32,
    /// Fewest queried affiliates for the check to apply; 0 = no minimum.
    pub min_sample: u32,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            percent: 50,
            min_sample: 4,
        }
    }
}

/// `Err(ValidatorUnavailable)` when the technical failures of `stats` exceed
/// `policy.percent` of the queried affiliates and at least `policy.min_sample`
/// were queried. Cache hits never count as queried.
pub fn check_availability(stats: &Stats, policy: &Policy) -> Result<(), DerechosError> {
    let queried = stats.queried;
    if policy.percent == 0 || queried == 0 || queried < policy.min_sample as usize {
        return Ok(());
    }
    if stats.unavailable * 100 <= policy.percent as usize * queried {
        return Ok(());
    }
    Err(DerechosError::ValidatorUnavailable {
        unavailable: stats.unavailable,
        queried,
        percent: policy.percent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(queried: usize, unavailable: usize) -> Stats {
        Stats {
            affiliates: queried,
            cached: 0,
            queried,
            unavailable,
        }
    }

    #[test]
    fn exceeding_the_share_with_enough_sample_is_an_error() {
        let err = check_availability(&stats(4, 3), &Policy::default()).expect_err("alert");
        assert_eq!(
            err.to_string(),
            "Validador de derechos no disponible: 3 de 4 afiliados consultados fallaron tecnicamente (75%), por encima del umbral de 50%; \
             las facturas quedaron en REVISION_MANUAL y solo se reintentan con --invoices cuando el validador responda"
        );
    }

    #[test]
    fn exactly_the_threshold_is_tolerated() {
        assert!(check_availability(&stats(4, 2), &Policy::default()).is_ok());
    }

    #[test]
    fn small_samples_and_disabled_policies_never_alert() {
        assert!(check_availability(&stats(3, 3), &Policy::default()).is_ok());
        let off = Policy {
            percent: 0,
            min_sample: 0,
        };
        assert!(check_availability(&stats(10, 10), &off).is_ok());
        assert!(
            check_availability(
                &stats(0, 0),
                &Policy {
                    percent: 1,
                    min_sample: 0
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn cache_hits_do_not_count_as_queried_and_stats_accumulate() {
        let mut total = Stats::default();
        total.add(Stats {
            affiliates: 3,
            cached: 2,
            queried: 1,
            unavailable: 1,
        });
        total.add(Stats {
            affiliates: 1,
            cached: 0,
            queried: 1,
            unavailable: 0,
        });
        assert_eq!(
            total,
            Stats {
                affiliates: 4,
                cached: 2,
                queried: 2,
                unavailable: 1
            }
        );
    }
}
