//! Test helpers for code that needs a stored dataset (feature `test-support`
//! and unit tests).
//!
//! ```no_run
//! # use tessia::{db::test_support::TestDb, stores::dataset::test_support::*};
//! # use tessia::domain::dataset::test_support::*;
//! let db = TestDb::new();
//! let store = tessia::stores::dataset::DatasetStore::new(db.pool());
//! let seq = seed_cargue(&db.pool(), "Convenio_21102026_21102026", 'a');
//! store.replace(&DatasetBuilder::new(seq).invoice(InvoiceBuilder::new("EVS1").line("10")).build(), false).unwrap();
//! ```

use crate::db::DbPool;
use crate::domain::cargue::load_context;
use crate::stores::cargues::{CarguesStore, NewCargue};

/// Registers an upload of `file_name` (a CSV name or stem) whose bytes hash to
/// 64 times `hash_char` (a lowercase hex digit) and returns its `cargues.seq`.
/// Another `hash_char` under the same name is a newer upload of the same load.
pub fn seed_cargue(pool: &DbPool, file_name: &str, hash_char: char) -> i64 {
    let info = load_context(file_name);
    let sha = hash_char.to_string().repeat(64);
    CarguesStore::new(pool.clone())
        .ingest(&NewCargue::from_load(&info, &sha))
        .map(|i| i.cargue.seq)
        .unwrap_or_else(|e| panic!("seed_cargue({file_name}): {e}"))
}
