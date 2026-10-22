//! Pure domain types and rules shared by all phases (no I/O).

pub mod cargue;
pub mod invoice;
pub mod phase;
pub mod radicacion;

pub use cargue::{LoadInfo, LoadOrigin};
pub use invoice::{normalize_invoice, valid_invoice};
pub use phase::{Outcome, Phase, PhaseState};
