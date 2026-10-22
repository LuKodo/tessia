//! Concatenation of PDFs (Java `PDFMergerUtility`, Go `pdfcpu` merge) and the
//! structural readers used by tests and diagnostics.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use lopdf::content::Content;
use lopdf::xref::XrefType;
use lopdf::{Dictionary, Document, Object, ObjectId};

/// Page attributes a page inherits from its ancestors; they must be copied
/// onto the page before it moves under a new `Pages` node.
const INHERITED: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

/// Concatenates `pdfs` in order into one document.
pub fn merge(pdfs: &[&[u8]]) -> Result<Vec<u8>> {
    if pdfs.is_empty() {
        bail!("soporte: merge PDF: nothing to merge");
    }
    let mut merged = Document::with_version("1.5");
    merged.reference_table.cross_reference_type = XrefType::CrossReferenceTable;
    let mut kids = Vec::new();
    let pages_id = merged.new_object_id();
    for (index, bytes) in pdfs.iter().enumerate() {
        let source = Document::load_mem(bytes).with_context(|| {
            format!(
                "soporte: merge PDF: part {} is not a readable PDF",
                index + 1
            )
        })?;
        append(&mut merged, source, pages_id, &mut kids)
            .with_context(|| format!("soporte: merge PDF: part {}", index + 1))?;
    }
    let mut pages = Dictionary::new();
    pages.set("Type", Object::Name(b"Pages".to_vec()));
    pages.set("Count", Object::Integer(kids.len() as i64));
    pages.set("Kids", Object::Array(kids));
    merged.objects.insert(pages_id, Object::Dictionary(pages));
    let mut catalog = Dictionary::new();
    catalog.set("Type", Object::Name(b"Catalog".to_vec()));
    catalog.set("Pages", Object::Reference(pages_id));
    let root = merged.add_object(catalog);
    merged.trailer.set("Root", Object::Reference(root));
    merged.prune_objects();
    let mut out = Vec::new();
    merged
        .save_to(&mut out)
        .context("soporte: merge PDF: write")?;
    Ok(out)
}

/// Moves the pages of `source` into `merged` (ids shifted), under `pages_id`.
fn append(
    merged: &mut Document,
    mut source: Document,
    pages_id: ObjectId,
    kids: &mut Vec<Object>,
) -> Result<()> {
    if source.is_encrypted() {
        bail!("encrypted PDF");
    }
    let page_ids: Vec<ObjectId> = source.get_pages().into_values().collect();
    if page_ids.is_empty() {
        bail!("PDF without pages");
    }
    for id in &page_ids {
        flatten_inheritance(&mut source, *id)?;
    }
    let offset = merged.max_id;
    let mut highest = offset;
    for (id, mut object) in std::mem::take(&mut source.objects) {
        shift_references(&mut object, offset);
        let new_id = (id.0 + offset, id.1);
        highest = highest.max(new_id.0);
        merged.objects.insert(new_id, object);
    }
    merged.max_id = highest;
    for id in page_ids {
        let new_id = (id.0 + offset, id.1);
        let Some(Object::Dictionary(page)) = merged.objects.get_mut(&new_id) else {
            bail!("page object missing");
        };
        page.set("Parent", Object::Reference(pages_id));
        kids.push(Object::Reference(new_id));
    }
    Ok(())
}

/// Copies the inherited attributes of the page's ancestors onto the page.
fn flatten_inheritance(doc: &mut Document, page_id: ObjectId) -> Result<()> {
    let mut inherited: BTreeMap<&[u8], Object> = BTreeMap::new();
    let mut parent = doc.get_dictionary(page_id)?.get(b"Parent").ok().cloned();
    let mut hops = 0;
    while let Some(Object::Reference(id)) = parent {
        hops += 1;
        if hops > 64 {
            bail!("page tree too deep");
        }
        let node = doc.get_dictionary(id)?;
        for key in INHERITED {
            if let (false, Ok(value)) = (inherited.contains_key(key), node.get(key)) {
                inherited.insert(key, value.clone());
            }
        }
        parent = node.get(b"Parent").ok().cloned();
    }
    let page = doc.get_dictionary_mut(page_id)?;
    for (key, value) in inherited {
        if !page.has(key) {
            page.set(key, value);
        }
    }
    Ok(())
}

fn shift_references(object: &mut Object, offset: u32) {
    match object {
        Object::Reference(id) => id.0 += offset,
        Object::Array(items) => items.iter_mut().for_each(|o| shift_references(o, offset)),
        Object::Dictionary(dict) => dict
            .iter_mut()
            .for_each(|(_, o)| shift_references(o, offset)),
        Object::Stream(stream) => stream
            .dict
            .iter_mut()
            .for_each(|(_, o)| shift_references(o, offset)),
        _ => {}
    }
}

/// Number of pages of a PDF.
pub fn page_count(pdf: &[u8]) -> Result<usize> {
    Ok(Document::load_mem(pdf)?.get_pages().len())
}

/// Per page, the strings drawn with `Tj`/`TJ`, joined by spaces (WinAnsi bytes
/// read as Latin-1). A structural check for tests and diagnostics, not a text
/// extractor: fonts with custom encodings come out as raw bytes.
pub fn page_texts(pdf: &[u8]) -> Result<Vec<String>> {
    let doc = Document::load_mem(pdf)?;
    let mut out = Vec::new();
    for (_, id) in doc.get_pages() {
        let content = Content::decode(&doc.get_page_content(id))?;
        let mut parts: Vec<String> = Vec::new();
        for op in content.operations {
            match op.operator.as_str() {
                "Tj" => parts.extend(op.operands.iter().filter_map(latin1)),
                "TJ" => {
                    for operand in &op.operands {
                        if let Object::Array(items) = operand {
                            parts.extend(items.iter().filter_map(latin1));
                        }
                    }
                }
                _ => {}
            }
        }
        out.push(parts.join(" "));
    }
    Ok(out)
}

fn latin1(object: &Object) -> Option<String> {
    match object {
        Object::String(bytes, _) => Some(bytes.iter().map(|b| char::from(*b)).collect()),
        _ => None,
    }
}

/// `(width, height)` of the MediaBox of each page, in points.
pub fn page_sizes(pdf: &[u8]) -> Result<Vec<(f64, f64)>> {
    let doc = Document::load_mem(pdf)?;
    let mut sizes = Vec::new();
    for (_, id) in doc.get_pages() {
        let mut node = doc.get_dictionary(id)?;
        let media = loop {
            if let Ok(media) = node.get(b"MediaBox") {
                break media.as_array()?.clone();
            }
            let parent = node.get(b"Parent")?.as_reference()?;
            node = doc.get_dictionary(parent)?;
        };
        let n: Vec<f64> = media
            .iter()
            .filter_map(|o| o.as_float().ok())
            .map(f64::from)
            .collect();
        if n.len() != 4 {
            bail!("bad MediaBox");
        }
        sizes.push((n[2] - n[0], n[3] - n[1]));
    }
    Ok(sizes)
}

/// Number of image XObjects referenced by each page's resources.
pub fn page_image_counts(pdf: &[u8]) -> Result<Vec<usize>> {
    let doc = Document::load_mem(pdf)?;
    Ok(doc
        .get_pages()
        .into_values()
        .map(|id| doc.get_page_images(id).map(|v| v.len()).unwrap_or(0))
        .collect())
}
