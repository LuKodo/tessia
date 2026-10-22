//! ZIP helpers for `<F>_XML_PDF.zip` (FEV-04, TF-07).

use std::io::{Cursor, Read, Write};

use zip::read::ZipFile;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use super::FevxmlError;

/// Cap on the bytes read from one ZIP entry (zip-bomb guard, FEV-04).
pub const MAX_ENTRY_BYTES: u64 = 256 << 20;

fn deflate(modified: Option<DateTime>) -> SimpleFileOptions {
    let o = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    match modified {
        Some(t) => o.last_modified_time(t),
        None => o,
    }
}

/// Entry name, falling back to a lossy decode of the raw bytes.
fn entry_name<R: Read>(f: &ZipFile<'_, R>) -> String {
    f.name()
        .map(|n| n.into_owned())
        .unwrap_or_else(|_| String::from_utf8_lossy(f.name_raw()).into_owned())
}

/// `strings.EqualFold`: case-insensitive comparison.
fn eq_fold(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

fn read_capped<R: Read>(r: R, name: &str) -> Result<Vec<u8>, FevxmlError> {
    read_limited(r, name, MAX_ENTRY_BYTES)
}

fn read_limited<R: Read>(r: R, name: &str, limit: u64) -> Result<Vec<u8>, FevxmlError> {
    let mut buf = Vec::new();
    r.take(limit + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > limit {
        return Err(FevxmlError::EntradaDemasiadoGrande(name.to_owned(), limit));
    }
    Ok(buf)
}

/// First file entry whose name ends in `.xml` (case-insensitive), or `None`.
/// An entry above [`MAX_ENTRY_BYTES`] is an error.
pub fn first_xml_entry(zip_bytes: &[u8]) -> Result<Option<Vec<u8>>, FevxmlError> {
    let mut ar = ZipArchive::new(Cursor::new(zip_bytes))?;
    for i in 0..ar.len() {
        let f = ar.by_index(i)?;
        let name = entry_name(&f);
        if f.is_dir() || !name.to_lowercase().ends_with(".xml") {
            continue;
        }
        return read_capped(f, &name).map(Some);
    }
    Ok(None)
}

/// First file entry whose base name equals `name` (case-insensitive, `\` and
/// `/` both separate directories), like `restaurarXmlDesdeZipXmlPdfSiExiste`.
pub fn read_entry(zip_bytes: &[u8], name: &str) -> Result<Option<Vec<u8>>, FevxmlError> {
    let mut ar = ZipArchive::new(Cursor::new(zip_bytes))?;
    for i in 0..ar.len() {
        let f = ar.by_index(i)?;
        let ename = entry_name(&f);
        let full = ename.replace('\\', "/");
        let base = full.rsplit('/').next().unwrap_or("");
        if f.is_dir() || !eq_fold(base, name) {
            continue;
        }
        return read_capped(f, &ename).map(Some);
    }
    Ok(None)
}

/// Builds `<F>_XML_PDF.zip`: the PDF entry first, then the XML, both Deflate
/// (TF-07). `modified` stamps both entries (the ZIP default when `None`).
pub fn build_xml_pdf_zip(
    pdf_name: &str,
    pdf: &[u8],
    xml_name: &str,
    xml: &[u8],
    modified: Option<DateTime>,
) -> Result<Vec<u8>, FevxmlError> {
    let mut zw = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in [(pdf_name, pdf), (xml_name, xml)] {
        zw.start_file(name, deflate(modified))?;
        zw.write_all(data)?;
    }
    Ok(zw.finish()?.into_inner())
}

/// `refrescarXmlEnZipXmlPdfSiExiste`: every entry named `name`
/// (case-insensitive) is dropped, the others are copied unchanged, and `data`
/// is appended last as `name` (Deflate).
pub fn replace_entry(
    zip_bytes: &[u8],
    name: &str,
    data: &[u8],
    modified: Option<DateTime>,
) -> Result<Vec<u8>, FevxmlError> {
    let mut ar = ZipArchive::new(Cursor::new(zip_bytes))?;
    let mut zw = ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..ar.len() {
        let f = ar.by_index_raw(i)?;
        if eq_fold(&entry_name(&f), name) {
            continue;
        }
        zw.raw_copy_file(f)?;
    }
    zw.start_file(name, deflate(modified))?;
    zw.write_all(data)?;
    Ok(zw.finish()?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zw = ZipWriter::new(Cursor::new(Vec::new()));
        for (n, d) in entries {
            zw.start_file(*n, deflate(None)).unwrap();
            zw.write_all(d).unwrap();
        }
        zw.finish().unwrap().into_inner()
    }

    fn names(z: &[u8]) -> Vec<String> {
        let ar = ZipArchive::new(Cursor::new(z)).unwrap();
        ar.file_names()
            .filter_map(|n| n.ok().map(|c| c.into_owned()))
            .collect()
    }

    #[test]
    fn entries_above_the_cap_are_rejected() {
        assert_eq!(read_limited(&[0u8; 4][..], "a.xml", 4).unwrap().len(), 4);
        assert!(matches!(
            read_limited(&[0u8; 5][..], "a.xml", 4),
            Err(FevxmlError::EntradaDemasiadoGrande(_, 4))
        ));
    }

    #[test]
    fn build_orders_pdf_then_xml_with_deflate() {
        let z = build_xml_pdf_zip("F1.pdf", b"%PDF", "F1.xml", b"<a/>", None).unwrap();
        assert_eq!(names(&z), ["F1.pdf", "F1.xml"]);
        let mut ar = ZipArchive::new(Cursor::new(&z[..])).unwrap();
        assert_eq!(
            ar.by_index(1).unwrap().compression(),
            CompressionMethod::Deflated
        );
    }

    #[test]
    fn first_xml_entry_skips_non_xml() {
        let z = zip_of(&[("F1.pdf", b"%PDF"), ("sub/A.XML", b"<a/>")]);
        assert_eq!(first_xml_entry(&z).unwrap().as_deref(), Some(&b"<a/>"[..]));
        let none = zip_of(&[("F1.pdf", b"%PDF")]);
        assert!(first_xml_entry(&none).unwrap().is_none());
        assert!(first_xml_entry(b"not a zip").is_err());
    }

    #[test]
    fn read_entry_matches_base_name_case_insensitively() {
        let z = zip_of(&[("d\\f1.XML", b"<x/>")]);
        assert_eq!(
            read_entry(&z, "F1.xml").unwrap().as_deref(),
            Some(&b"<x/>"[..])
        );
        assert!(read_entry(&z, "F2.xml").unwrap().is_none());
    }

    #[test]
    fn replace_entry_keeps_others_and_appends_xml_last() {
        let z = zip_of(&[("F1.xml", b"old"), ("F1.pdf", b"%PDF"), ("f1.XML", b"dup")]);
        let out = replace_entry(&z, "F1.xml", b"new", None).unwrap();
        assert_eq!(names(&out), ["F1.pdf", "F1.xml"]);
        assert_eq!(
            read_entry(&out, "F1.xml").unwrap().as_deref(),
            Some(&b"new"[..])
        );
        assert_eq!(
            read_entry(&out, "F1.pdf").unwrap().as_deref(),
            Some(&b"%PDF"[..])
        );
    }
}
