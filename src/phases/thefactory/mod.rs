//! `thefactory descargar`: invoice XML/PDF/ZIP and CPScan support download.
//!
//! Port of Go `cmd/tessia/cmd_thefactory.go` (docs/thefactory.md). For each
//! invoice the phase writes, through [`Artifacts`](crate::storage::Artifacts)
//! (relative paths exactly as in the Go folder layout):
//!
//! | artifact | content |
//! |---|---|
//! | `<F>.xml` | the DIAN `AttachedDocument`, after [`fevxml::normalize_download`](crate::fevxml::normalize_download) |
//! | `<F>.pdf` | the PDF (>= 1024 bytes, `%PDF-` header) |
//! | `XML/<F>_XML_PDF.zip` | entries `<F>.pdf` then `<F>.xml` |
//! | `CPSCAN/<F>.support.<pdf\|jpg\|png\|gif\|bmp\|tif>` | the CPScan support (only when there is one) |
//! | `CPSCAN/<F>.cpscan.json` | the CPScan sidecar, published last |
//!
//! # CPScan sidecar (contract with `soporte generar`)
//!
//! `soporte generar` reads `CPSCAN/<F>.cpscan.json` from the artifact store
//! without calling this code, so the format is documented on
//! [`CpscanSidecar`]: a JSON object with `version` (always `1`), `ssc`,
//! `documento` (bool), `origen`, `adicional`, `estado`, `archivo` (`""` = CPScan
//! has nothing, otherwise the downloaded reference), `ext` and `consultado`
//! (RFC 3339 UTC). A missing, unparsable or payload-less sidecar means the
//! lookup is not done; a transient failure never writes one.
//!
//! # State (phase `thefactory`)
//!
//! Invoices are claimed in `proceso_estado` with `claim_available`, so several
//! machines split the work. XML + PDF + ZIP present = `DONE` (the motivo is
//! `CPSCAN sin soporte (SSC ..)` / `CPSCAN pendiente (SSC ..): ..` when CPScan
//! has nothing or failed transiently, which never changes the outcome);
//! anything missing = `FAILED` with the status line as motivo. Complete
//! invoices are never downloaded again (`YA_EXISTE`).
//!
//! Configuration: `thefactory.base_url|token_empresa|token_password|ca_file`,
//! `cpscan.base_url|bucket_url|ca_file|tls_server_name`.

pub mod cli;
pub mod client;
pub mod cpscan;
mod inputs;
mod invoice;
mod plan;
mod runner;
mod runtime;

use anyhow::{Result, bail};

use crate::context::AppContext;
use crate::phases::common::{PhaseReport, PhaseRequest};

pub use cli::Args;
pub use cpscan::CpscanSidecar;
pub use plan::TfInput;
pub use runner::{DEFAULT_WORKERS, run_inputs};
pub use runtime::Runtime;

/// Entry point of the command line.
pub fn run(ctx: &AppContext, args: Args) -> Result<()> {
    let cli::Command::Descargar(args) = args.command;
    let request = args.common.to_request()?;
    run_phase(ctx, &request).map(|_| ())
}

/// Runs the phase for the invoices of `req` (called by `proceso run`).
pub fn run_phase(ctx: &AppContext, req: &PhaseRequest) -> Result<PhaseReport> {
    let workers = match req.workers {
        Some(n) if !(1..=DEFAULT_WORKERS).contains(&n) => {
            bail!("--workers debe estar entre 1 y {DEFAULT_WORKERS}")
        }
        Some(n) => n,
        None => DEFAULT_WORKERS,
    };
    let (inputs, note) = inputs::resolve(ctx, req)?;
    if let Some(note) = note {
        println!("thefactory: dataset not readable, the CPScan part is skipped ({note})");
    }
    run_inputs(ctx, &req.cargue_id, inputs, workers, None)
}
