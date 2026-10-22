//! The `CRC_<nit>_<F>.zip` container: one Deflate entry named
//! `CRC_<nit>_<F>.pdf` (entry name, order and method match the Java block).

use std::io::{Cursor, Write};

use anyhow::Result;
use chrono::{Datelike, Local, Timelike};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

/// `CRC_<nit>_<F>.zip` (the step 4 file name).
pub fn zip_name(nit: &str, invoice: &str) -> String {
    format!("CRC_{nit}_{invoice}.zip")
}

/// `CRC_<nit>_<F>.pdf`, the only entry of the ZIP.
pub fn pdf_name(nit: &str, invoice: &str) -> String {
    format!("CRC_{nit}_{invoice}.pdf")
}

/// Artifact path of the ZIP: `CRC/CRC_<nit>_<F>.zip`.
pub fn zip_rel(nit: &str, invoice: &str) -> String {
    format!("CRC/{}", zip_name(nit, invoice))
}

/// The modification time of "now" in the ZIP's DOS format (local time, like Java).
pub fn now() -> Option<DateTime> {
    let t = Local::now();
    DateTime::from_date_and_time(
        u16::try_from(t.year()).ok()?,
        u8::try_from(t.month()).ok()?,
        u8::try_from(t.day()).ok()?,
        u8::try_from(t.hour()).ok()?,
        u8::try_from(t.minute()).ok()?,
        u8::try_from(t.second()).ok()?,
    )
    .ok()
}

/// One Deflate entry `entry` holding `pdf`.
pub fn build_zip(entry: &str, pdf: &[u8], modified: Option<DateTime>) -> Result<Vec<u8>> {
    let mut options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    if let Some(modified) = modified {
        options = options.last_modified_time(modified);
    }
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer.start_file(entry, options)?;
    writer.write_all(pdf)?;
    Ok(writer.finish()?.into_inner())
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use zip::ZipArchive;

    use super::*;

    #[test]
    fn names_follow_the_step_4_convention() {
        assert_eq!(zip_name("806011811", "FEV1"), "CRC_806011811_FEV1.zip");
        assert_eq!(pdf_name("806011811", "FEV1"), "CRC_806011811_FEV1.pdf");
        assert_eq!(zip_rel("806011811", "FEV1"), "CRC/CRC_806011811_FEV1.zip");
    }

    #[test]
    fn the_zip_has_one_deflate_entry_with_the_pdf() {
        let pdf = b"%PDF-1.5 fake ".repeat(100);
        let bytes = build_zip("CRC_1_FEV1.pdf", &pdf, now()).expect("zip");
        let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("open");
        assert_eq!(archive.len(), 1);
        let mut entry = archive.by_index(0).expect("entry");
        assert_eq!(entry.name().expect("name"), "CRC_1_FEV1.pdf");
        assert_eq!(entry.compression(), CompressionMethod::Deflated);
        assert!(entry.compressed_size() < entry.size());
        let mut out = Vec::new();
        entry.read_to_end(&mut out).expect("read");
        assert_eq!(out, pdf);
    }
}
