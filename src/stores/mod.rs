//! Postgres stores (Diesel). Each store lives in its own directory with its
//! own hand-written `schema.rs` (`diesel::table!`), so phase writers add
//! stores without touching each other's files.
//!
//! Conventions every store follows:
//! * `Store::new(DbPool)`; methods take `&self` and return [`DbResult`];
//! * single idempotent statements go through `DbPool::run`, multi-statement
//!   units through `DbPool::transaction` (both retry transient failures);
//! * timestamps are `TIMESTAMPTZ` set by the database clock (`now()`), so
//!   machines with skewed clocks agree; Rust sees `DateTime<Utc>`;
//! * money is `NUMERIC` / `rust_decimal::Decimal`;
//! * the tests run against a real database through `TestDb`.
//!
//! [`DbResult`]: crate::db::DbResult

pub mod artefactos;
pub mod cargues;
pub mod derechos;
pub mod leases;
pub mod parametros;
pub mod proceso;
pub mod radicacion;
pub mod valor_xml;

/// Splits `items` into slices of at most [`CHUNK`] elements; used to keep
/// `IN (...)` lists and batches well under Postgres' 65,535 bind limit.
pub fn chunks<T>(items: &[T]) -> std::slice::Chunks<'_, T> {
    items.chunks(CHUNK)
}

/// Maximum items per batched statement.
pub const CHUNK: usize = 300;
