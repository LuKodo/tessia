//! Short leases around the one-off steps that must not run on two machines at
//! once (preparing a cargue, downloading the day's CSV). They are never held
//! while invoices are processed: that work is split between machines by the
//! per-(invoice, phase) claims.

use std::sync::atomic::{AtomicBool, Ordering};
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
    let lost = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let heartbeat = scope.spawn(|| {
            let mut errors = 0u32;
            while let Err(crossbeam_channel::RecvTimeoutError::Timeout) =
                stopped.recv_timeout(RENEW_EVERY)
            {
                match guard.renew() {
                    Ok(true) => errors = 0,
                    Ok(false) => {
                        tracing::warn!(lease = %name, "the lease was lost");
                        lost.store(true, Ordering::SeqCst);
                        break;
                    }
                    Err(err) => {
                        errors += 1;
                        tracing::warn!(lease = %name, error = %err, "could not renew the lease");
                        if errors * RENEW_EVERY.as_secs() as u32 >= TTL.as_secs() as u32 {
                            tracing::warn!(lease = %name, "lease renewals failing for over the TTL; treating the lease as lost");
                            lost.store(true, Ordering::SeqCst);
                            break;
                        }
                    }
                }
            }
        });
        let result = work();
        drop(stop);
        let _ = heartbeat.join();
        if lost.load(Ordering::SeqCst) && result.is_ok() {
            bail!(
                "la máquina perdió el lease «{name}» mientras {what}; otra máquina pudo tomar el trabajo, así que este resultado no se usa"
            );
        }
        result
    })
}
