//! Pure-Rust PDF rendering of the support pages and the PDF merge.
//!
//! * `canvas`: page builder on `lopdf` (Letter page, fonts, text, boxes, images);
//! * `formula`: formula image page (aspect-fit at the top-left);
//! * `delivery`: the `Formato_App` document;
//! * `signature`: the patient signature image drawn from the name;
//! * `merge`: concatenation of PDFs and structural readers for tests.

pub mod canvas;
pub mod delivery;
pub mod font;
pub mod formula;
mod helvetica;
pub mod image;
pub mod merge;
pub mod signature;
