//! One module directory per command, so each phase writer only touches its own.
//!
//! | module | Go command | `run_phase` (used by `proceso run`) |
//! |---|---|---|
//! | [`convenio_descargar`] | `convenio descargar` | no |
//! | [`convenio_preparar`] | `convenio preparar` | yes |
//! | [`thefactory`] | `thefactory descargar` | yes |
//! | [`derechos`] | `derechos validar` | yes |
//! | [`fev`] | `fev ajustar`, `fev verificar` | `run_ajustar_phase`, `run_verificar_phase` |
//! | [`conciliar`] | `conciliar` | yes |
//! | [`rips`] | `rips generar` | yes |
//! | [`cuv`] | `cuv generar`, `cuv barrer` | yes (`generar`) |
//! | [`soporte`] | `soporte generar` | yes |
//! | [`radicacion`] | `radicacion run`, `radicacion recheck-step2` | yes |
//! | [`proceso`] | `proceso run` | no (it is the caller) |
//!
//! Each module has `cli.rs` (the clap `Args`, already wired in `crate::cli`)
//! and `mod.rs` with `run(ctx, args)` for the command line and
//! `run_phase(ctx, &PhaseRequest) -> PhaseReport` for the orchestrator.
//! See [`common`] for the contract.

pub mod common;

pub mod conciliar;
pub mod convenio_descargar;
pub mod convenio_preparar;
pub mod cuv;
pub mod derechos;
pub mod fev;
pub mod proceso;
pub mod radicacion;
pub mod rips;
pub mod soporte;
pub mod thefactory;
