//! Short leases around the one-off steps that must not run on two machines at
//! once (preparing a cargue, downloading the day's CSV). They are never held
//! while invoices are processed: that work is split between machines by the
//! per-(invoice, phase) claims.

use std::time::{Duration, Instant};

use anyhow::{Result, bail};

use crate::context::AppContext;

/// How long a holder may go without renewing.
pub const TTL: Duration = Duration::from_secs(10 * 60);
const RENEW_EVERY: Duration = Duration::from_secs(2 * 60);

/// Runs `work` holding the lease `name`, renewed in the background. Waits up
/// to `wait` (polling every `poll`) while another owner holds it.
pub fn with_lease<T: Send>(
    ctx: &AppContext,
    name: &str,
    what: &str,
    wait: Duration,
    poll: Duration,
    work: impl FnOnce() -> Result<T> + Send,
) -> Result<T> {
    let leases = ctx.leases();
    let deadline = Instant::now() + wait;
    let guard = loop {
        if let Some(guard) = leases.guard(name, &ctx.owner, TTL)? {
            break guard;
        }
        if Instant::now() >= deadline || crate::shutdown::requested() {
            let holder = leases.holder(name).ok().flatten().unwrap_or_default();
            bail!(
                "otra máquina está {what} (lease «{name}» en poder de {holder}); se agotó la espera"
            );
        }
        std::thread::sleep(poll);
    };
    let (stop, stopped) = crossbeam_channel::bounded::<()>(1);
    std::thread::scope(|scope| {
        let heartbeat = scope.spawn(|| {
            while let Err(crossbeam_channel::RecvTimeoutError::Timeout) =
                stopped.recv_timeout(RENEW_EVERY)
            {
                match guard.renew() {
                    Ok(true) => {}
                    Ok(false) => tracing::warn!(lease = %name, "the lease was lost"),
                    Err(err) => {
                        tracing::warn!(lease = %name, error = %err, "could not renew the lease")
                    }
                }
            }
        });
        let result = work();
        drop(stop);
        let _ = heartbeat.join();
        result
    })
}
