//! `conciliar`: rebuilds the convenio lines of the invoices whose XML total
//! differs from the convenio (XML lines + the Medicar formula date).
//!
//! * [`rules`]: pure analysis and rebuild (CONC-05, 06, 09);
//! * [`plan`]: per-invoice decisions, Medicar failure mapping, exit rules
//!   (CONC-04, 06, 08, 14);
//! * [`medicar`] + [`portal_html`]: the formula-date client (CONC-07);
//! * [`catalog`]: the nota técnica view built from the pinned NT version;
//! * [`files`]: invoice XML, CUV precheck and stale-output clean-up in the
//!   bucket (CONC-08, 11);
//! * [`run`]: dataset reads/updates, claims, leases, recording and the
//!   follow-up (CONC-01..03, 10, 13).

pub mod catalog;
pub mod cli;
#[cfg(any(test, feature = "test-support"))]
pub mod fake_portal;
pub mod files;
pub mod medicar;
pub mod plan;
pub mod portal_html;
pub mod rules;
pub mod run;

#[cfg(test)]
mod medicar_tests;
#[cfg(test)]
mod plan_tests;
#[cfg(test)]
mod rules_tests;

pub use cli::Args;
pub use run::{
    ConciliarOptions, Execution, lease_name, run, run_phase, run_phase_detailed, run_request,
};
