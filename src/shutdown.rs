//! Cooperative Ctrl-C handling.
//!
//! The first Ctrl-C (or SIGTERM) sets a flag that long loops poll with
//! [`requested`] so they can stop starting new work, release their claims and
//! let their `Workdir` drop; a second one exits immediately with status 130.

use std::sync::atomic::{AtomicBool, Ordering};

static REQUESTED: AtomicBool = AtomicBool::new(false);

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

/// True once an interrupt was received.
pub fn requested() -> bool {
    REQUESTED.load(Ordering::SeqCst)
}
