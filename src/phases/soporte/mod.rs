//! `soporte generar`: builds `CRC/CRC_<nit>_<F>.zip`, the delivery support of
//! each invoice (support page(s) followed by the invoice PDF).
//!
//! Layout:
//!
//! * `input`: the per-invoice input (`SoporteInput`) and the Go normalization rules;
//! * `dataset`: adapter from the prepared dataset (`ctx.dataset()`) to the input;
//! * `cpscan`: reader of the `CPSCAN/*` sidecar left by `thefactory descargar`;
//! * `decide`: result codes and the branch decision;
//! * `generator`: assembles the support PDF (no I/O);
//! * `render`: pure-Rust PDF pages (formula image, delivery page, signature) and merge;
//! * `assets`: shared signature files and logo (bucket artifacts of `_shared`);
//! * `zipfile`: the ZIP container;
//! * `run`: claims, workers and state recording.

//!
//! # Mapping to the Postgres/bucket world (S3, S5) and deviations from Go
//!
//! * **Inputs/outputs are artifacts of the cargue** (never a folder): reads `<F>.pdf`, `CPSCAN/<F>.cpscan.json`
//!   and `CPSCAN/<F>.support.<ext>`; publishes `CRC/CRC_<nit>_<F>.zip` (one Deflate entry `CRC_<nit>_<F>.pdf`:
//!   support page(s) followed by `<F>.pdf`). "Exists" is a non-empty indexed artifact; `--regenerar` replaces
//!   it only after the new one is built, so a failed rebuild keeps the old ZIP (`ZIP anterior conservado`).
//! * **Exclusion** between machines: `proceso_estado` claims (`claim_available`, an invoice another machine
//!   owns is skipped as `EN_PROCESO_OTRA_MAQUINA`) plus a per-invoice lease `soporte:<cargue>:<F>` held while
//!   building. They replace `.tessia-soporte.lock`. A bucket/database failure releases the claim and is
//!   reported (the command exits non-zero) instead of storing a state.
//! * **Shared files** (`paths.firmas`, `paths.reportes`): signatures are bucket artifacts
//!   `firmas/<tipodoc><numerodoc>.png` of the reserved cargue id `_shared` (flat key
//!   `_shared__firmas__<name>.png`) and the logo is `_shared`/`reportes/logo.png`; the legacy folders are still
//!   read (read only) when configured, and the logo falls back to a synthetic placeholder embedded in the
//!   binary (`assets/soporte/logo.png`): upload the real logo to the bucket. See `assets`.
//! * **Rendering is pure Rust**: `lopdf` builds and merges the PDFs (one dependency for both; low-level
//!   drawing with explicit content streams keeps the Go geometry exact), `image` decodes
//!   PNG/JPEG/GIF/BMP/TIFF, `ab_glyph` rasterizes the signature with the Go Italic font embedded with
//!   `include_bytes!` (BSD-style license), and the Helvetica/Helvetica-Bold advance widths (Adobe AFM) are a
//!   generated table, so no font is embedded in the PDFs. PDFs are not byte-identical to Java/Go; tests
//!   compare structure (page count and size, images, drawn text).
//! * **No JVM/Jasper sidecar**: `Formato_App_Deposito` has no renderer (it never existed) and is
//!   `FORMATO_DEPOSITO_NO_DISPONIBLE`; the report names are internal, so "unknown report" cannot occur.
//!   `soporte.java_bin` and `soporte.sidecar_classpath` are not read.
//! * No dry-run, no confirmation (S1). The hint texts name `facturacion thefactory descargar`.

pub mod assets;
pub mod cli;
pub mod cpscan;
pub mod dataset;
pub mod decide;
pub mod generator;
pub mod input;
pub mod render;
pub mod run;
pub mod zipfile;

use anyhow::{Result, bail};

use crate::context::AppContext;
use crate::phases::common::{PhaseReport, PhaseRequest};

pub use cli::Args;
pub use input::{SoporteInput, SoporteRow};
pub use run::{Options, SoporteRun, run_inputs};

/// Entry point of the command line.
pub fn run(ctx: &AppContext, args: Args) -> Result<()> {
    let cli::Command::Generar(generar) = args.command;
    let req = generar.common.to_request()?;
    let options = Options {
        regenerate: generar.regenerar,
    };
    let inputs = dataset::load_inputs(ctx, &req)?;
    let result = run_inputs(ctx, &req, options, inputs)?;
    for line in &result.lines {
        println!("{line}");
    }
    println!("{}", result.summary_line());
    if !result.errors.is_empty() {
        bail!(
            "soporte: {} factura(s) con error de infraestructura: {}",
            result.errors.len(),
            result.errors.join("; ")
        );
    }
    Ok(())
}

/// Runs the phase for the invoices of `req` (called by `proceso run`; it never regenerates).
pub fn run_phase(ctx: &AppContext, req: &PhaseRequest) -> Result<PhaseReport> {
    let inputs = dataset::load_inputs(ctx, req)?;
    Ok(run_inputs(ctx, req, Options::default(), inputs)?.report)
}
