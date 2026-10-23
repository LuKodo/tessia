//! Cooperative Ctrl-C handling.
//!
//! The first Ctrl-C (or SIGTERM) sets a flag that long loops poll with
//! [`requested`] so they can stop starting new work, release their claims and
//! let their `Workdir` drop; a second one exits immediately with status 130.
//!
//! The dashboard's job runner (one job at a time per process) adds a second,
//! resettable flag, the *job cancel*: [`cancel_job`] raises it, [`reset_job`]
//! lowers it, and [`requested`] reports either, so every loop that already
//! polls [`requested`] also stops when the operator cancels the running job.
//! The job flag never touches the Ctrl-C one.

use std::sync::atomic::{AtomicBool, Ordering};

static REQUESTED: AtomicBool = AtomicBool::new(false);
static JOB_CANCELLED: AtomicBool = AtomicBool::new(false);

/// Installs the handler (idempotent: later calls are ignored).
pub fn install() {
    let handler = ctrlc::set_handler(|| {
        if REQUESTED.swap(true, Ordering::SeqCst) {
            std::process::exit(130);
        }
        eprintln!("Interrumpido: terminando el trabajo en curso (Ctrl-C otra vez para salir ya)");
    });
    if let Err(err) = handler {
        tracing::debug!(error = %err, "ctrl-c handler not installed");
    }
}

/// True once an interrupt was received or the running job was cancelled.
pub fn requested() -> bool {
    REQUESTED.load(Ordering::SeqCst) || JOB_CANCELLED.load(Ordering::SeqCst)
}

/// Asks the running job to stop: [`requested`] is true until [`reset_job`].
pub fn cancel_job() {
    JOB_CANCELLED.store(true, Ordering::SeqCst);
}

/// Lowers the job cancel flag (a new job starts, or the cancelled one ended).
pub fn reset_job() {
    JOB_CANCELLED.store(false, Ordering::SeqCst);
}
