//! Retry of units of work that failed with a transient Postgres error.

use std::time::Duration;

use rand::Rng;

use super::error::{DbError, DbResult};

/// Total runs of one unit of work (the first plus the retries).
pub const MAX_ATTEMPTS: u32 = 4;

/// Jittered backoff before retry number `attempt` (1-based): about 50 ms,
/// 100 ms, 200 ms plus up to the same amount of random jitter.
pub fn backoff(attempt: u32) -> Duration {
    let base = Duration::from_millis(50) * 2u32.saturating_pow(attempt.saturating_sub(1));
    let jitter = rand::rng().random_range(0..=base.as_millis() as u64);
    base + Duration::from_millis(jitter)
}

/// Runs `unit`, running it again (up to [`MAX_ATTEMPTS`] runs in total) while
/// it fails with a transient error (see [`DbError::is_transient`]).
///
/// `unit` must be safe to repeat: it either leaves nothing committed when it
/// fails (a transaction) or converges when it already committed (an upsert, a
/// compare-and-set). `op` names the operation in the retry log.
pub fn with_retry<T>(op: &str, mut unit: impl FnMut() -> DbResult<T>) -> DbResult<T> {
    with_retry_sleeping(op, &mut unit, std::thread::sleep)
}

fn with_retry_sleeping<T>(
    op: &str,
    unit: &mut dyn FnMut() -> DbResult<T>,
    sleep: impl Fn(Duration),
) -> DbResult<T> {
    let mut attempt = 1;
    loop {
        match unit() {
            Err(err) if err.is_transient() && attempt < MAX_ATTEMPTS => {
                let delay = backoff(attempt);
                tracing::warn!(op, attempt, delay_ms = delay.as_millis() as u64, error = %err, "transient database error, retrying");
                sleep(delay);
                attempt += 1;
            }
            Err(err) => return Err(annotate(err, op, attempt)),
            Ok(value) => return Ok(value),
        }
    }
}

fn annotate(err: DbError, op: &str, attempts: u32) -> DbError {
    if err.is_transient() {
        DbError::Other(format!(
            "{op}: la base de datos no respondió tras {attempts} intentos: {err}"
        ))
    } else {
        err
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use diesel::result::{DatabaseErrorKind, Error as DieselError};

    use super::*;

    fn transient() -> DbError {
        DbError::Query(DieselError::DatabaseError(
            DatabaseErrorKind::SerializationFailure,
            Box::new("x".to_owned()),
        ))
    }

    #[test]
    fn retries_transient_errors_then_succeeds() {
        let calls = Cell::new(0);
        let result = with_retry_sleeping(
            "t",
            &mut || {
                calls.set(calls.get() + 1);
                if calls.get() < 3 {
                    Err(transient())
                } else {
                    Ok(calls.get())
                }
            },
            |_| {},
        );
        assert_eq!(result.expect("eventually ok"), 3);
    }

    #[test]
    fn gives_up_after_the_attempt_budget() {
        let calls = Cell::new(0);
        let result: DbResult<()> = with_retry_sleeping(
            "t",
            &mut || {
                calls.set(calls.get() + 1);
                Err(transient())
            },
            |_| {},
        );
        assert_eq!(calls.get(), MAX_ATTEMPTS);
        assert!(
            result
                .expect_err("exhausted")
                .to_string()
                .contains("tras 4 intentos")
        );
    }

    #[test]
    fn does_not_retry_permanent_errors() {
        let calls = Cell::new(0);
        let result: DbResult<()> = with_retry_sleeping(
            "t",
            &mut || {
                calls.set(calls.get() + 1);
                Err(DbError::other("bad"))
            },
            |_| {},
        );
        assert_eq!(calls.get(), 1);
        assert!(result.is_err());
    }

    #[test]
    fn backoff_grows_and_stays_bounded() {
        assert!(
            backoff(1) >= Duration::from_millis(50) && backoff(1) <= Duration::from_millis(100)
        );
        assert!(backoff(3) >= Duration::from_millis(200));
    }
}
