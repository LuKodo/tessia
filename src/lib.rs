//! Tessia: Rust port of the Go billing CLI, on Postgres, an object store and a
//! read-only Redis Nota Técnica. See `docs/architecture.md`.

pub mod api;
pub mod cli;
pub mod config;
pub mod context;
pub mod db;
pub mod desktop;
pub mod domain;
pub mod fevxml;
pub mod http;
pub mod jobs;
pub mod nt;
pub mod phases;
pub mod shutdown;
pub mod storage;
pub mod stores;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
pub mod timing;
pub mod util;
