//! Test helpers for unit tests and `tests/*.rs` (feature `test-support`,
//! enabled automatically for this crate's own tests):
//!
//! * [`MockServer`]: a scripted local HTTP server (`tiny_http`) that records
//!   the requests it receives; every client is tested against it, never
//!   against the real systems;
//! * `crate::db::test_support::TestDb`: a throwaway migrated Postgres database.

mod mock_http;

pub use mock_http::{MockResponse, MockServer, RecordedRequest};
