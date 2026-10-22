//! A tiny Letter-page PDF builder on top of `lopdf`.
//!
//! Coordinates are page-top ones (y grows downwards, like the JRXML the
//! delivery page comes from); the conversion to PDF space stays here.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use anyhow::{Context, Result};
use lopdf::xref::XrefType;
use lopdf::{Dictionary, Document, Object, Stream};

use super::font;
use super::image::{self, Accept, Embedded};

pub const PAGE_WIDTH: f64 = 612.0;
pub const PAGE_HEIGHT: f64 = 792.0;

/// A rectangle in page-top coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Frame {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Frame {
        Frame {
            x,
            y,
            width,
            height,
        }
    }
}

/// Horizontal alignment of a text cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
}

/// Two-decimal half-up rounding (Jasper's coordinate precision).
pub fn half_up(v: f64) -> f64 {
    (v * 100.0 + 0.5).floor() / 100.0
}

pub struct Canvas {
    doc: Document,
    pages: Vec<String>,
    content: String,
    images: BTreeMap<String, Embedded>,
}

impl Canvas {
    pub fn new() -> Canvas {
        let mut doc = Document::with_version("1.5");
        doc.reference_table.cross_reference_type = XrefType::CrossReferenceTable;
        Canvas {
            doc,
            pages: Vec::new(),
            content: String::new(),
            images: BTreeMap::new(),
        }
    }

    /// Four separate 0.25 pt butt-cap segments (top, left, bottom, right), each
    /// extended by half the pen width, with two-decimal half-up coordinates.
    pub fn rule_box(&mut self, f: Frame) {
        const HALF: f64 = 0.125;
        let (left, right, top, bottom) = (
            f.x,
            f.x + f.width,
            PAGE_HEIGHT - f.y,
            PAGE_HEIGHT - f.y - f.height,
        );
        for s in [
            [left - HALF, top, right + HALF, top],
            [left, top + HALF, left, bottom - HALF],
            [left - HALF, bottom, right + HALF, bottom],
            [right, top + HALF, right, bottom - HALF],
        ] {
            let _ = writeln!(
                self.content,
                "0.25 w 0 J 0 0 0 RG [] 0 d {:.2} {:.2} m {:.2} {:.2} l S",
                half_up(s[0]),
                half_up(s[1]),
                half_up(s[2]),
                half_up(s[3])
            );
        }
    }

    /// Text at a fixed baseline, clipped to its frame.
    pub fn text(
        &mut self,
        text: &str,
        f: Frame,
        size: f64,
        baseline: f64,
        bold: bool,
        align: Align,
    ) {
        let mut x = f.x;
        if align == Align::Center {
            // Jasper computes in float32 (502.495 becomes 502.49); so does this.
            let width = font::text_width(text, bold, size) as f32;
            x = half_up(f64::from(f.x as f32 + (f.width as f32 - width) / 2.0));
        }
        let name = if bold { "F2" } else { "F1" };
        let _ = writeln!(
            self.content,
            "q {:.5} {:.5} {:.5} {:.5} re W n BT /{name} {size} Tf {x:.5} {:.5} Td {} Tj ET Q",
            f.x,
            PAGE_HEIGHT - f.y - f.height,
            f.width,
            f.height,
            PAGE_HEIGHT - baseline,
            font::pdf_literal(&font::encode(text)),
        );
    }

    /// Justified static text with explicit word/character spacing.
    pub fn spaced_text(
        &mut self,
        text: &str,
        f: Frame,
        size: f64,
        baseline: f64,
        word: f64,
        chars: f64,
    ) {
        let _ = writeln!(
            self.content,
            "q {:.5} {:.5} {:.5} {:.5} re W n BT /F1 {size:.2} Tf {word:.5} Tw {chars:.5} Tc 1 0 0 1 {:.5} {:.5} Tm {} Tj ET Q",
            f.x,
            PAGE_HEIGHT - f.y - f.height,
            f.width,
            f.height,
            f.x,
            PAGE_HEIGHT - baseline,
            font::pdf_literal(&font::encode(text)),
        );
    }

    /// Draws an image in its stored colors. Each `key` is embedded once (the
    /// signature repeats on every page). With `fit` the image is scaled to fit
    /// the frame and centered horizontally.
    pub fn image(&mut self, data: &[u8], key: &str, f: Frame, fit: bool) -> Result<()> {
        let embedded = self.embedded(data, key)?;
        let (mut x, y, mut w, mut h) = (f.x, f.y, f.width, f.height);
        if fit {
            let scale = (w / f64::from(embedded.width)).min(h / f64::from(embedded.height));
            w = f64::from(embedded.width) * scale;
            h = f64::from(embedded.height) * scale;
            x += (f.width - w) / 2.0;
        }
        let _ = writeln!(
            self.content,
            "q {w:.5} 0 0 {h:.5} {x:.5} {:.5} cm /{key} Do Q",
            PAGE_HEIGHT - y - h
        );
        Ok(())
    }

    /// Draws an image of `width` x `height` points with its top-left corner at `(x, y)`.
    pub fn image_at(&mut self, data: &[u8], key: &str, accept: Accept, f: Frame) -> Result<()> {
        self.embedded_with(data, key, accept)?;
        let _ = writeln!(
            self.content,
            "q {:.5} 0 0 {:.5} {:.5} {:.5} cm /{key} Do Q",
            f.width,
            f.height,
            f.x,
            PAGE_HEIGHT - f.y - f.height
        );
        Ok(())
    }

    fn embedded(&mut self, data: &[u8], key: &str) -> Result<Embedded> {
        self.embedded_with(data, key, Accept::PngJpeg)
    }

    fn embedded_with(&mut self, data: &[u8], key: &str, accept: Accept) -> Result<Embedded> {
        if let Some(known) = self.images.get(key) {
            return Ok(*known);
        }
        let embedded = image::embed(&mut self.doc, data, accept)?;
        self.images.insert(key.to_owned(), embedded);
        Ok(embedded)
    }

    /// Closes the page being drawn; the next drawing call starts a new one.
    pub fn end_page(&mut self) {
        self.pages.push(std::mem::take(&mut self.content));
    }

    /// The finished document.
    pub fn finish(mut self) -> Result<Vec<u8>> {
        if !self.content.is_empty() {
            self.end_page();
        }
        if self.pages.is_empty() {
            anyhow::bail!("no pages drawn");
        }
        let pages_id = self.doc.new_object_id();
        let resources = self.resources();
        let mut kids = Vec::with_capacity(self.pages.len());
        for content in std::mem::take(&mut self.pages) {
            let mut stream = Stream::new(Dictionary::new(), content.into_bytes());
            stream.compress().context("compress page content")?;
            let contents = self.doc.add_object(stream);
            let mut page = Dictionary::new();
            page.set("Type", Object::Name(b"Page".to_vec()));
            page.set("Parent", Object::Reference(pages_id));
            page.set("MediaBox", media_box());
            page.set("Resources", Object::Dictionary(resources.clone()));
            page.set("Contents", Object::Reference(contents));
            kids.push(Object::Reference(self.doc.add_object(page)));
        }
        let mut pages = Dictionary::new();
        pages.set("Type", Object::Name(b"Pages".to_vec()));
        pages.set("Count", Object::Integer(kids.len() as i64));
        pages.set("Kids", Object::Array(kids));
        self.doc.objects.insert(pages_id, Object::Dictionary(pages));
        let mut catalog = Dictionary::new();
        catalog.set("Type", Object::Name(b"Catalog".to_vec()));
        catalog.set("Pages", Object::Reference(pages_id));
        let root = self.doc.add_object(catalog);
        self.doc.trailer.set("Root", Object::Reference(root));
        let mut out = Vec::new();
        self.doc.save_to(&mut out).context("write pdf")?;
        Ok(out)
    }

    fn resources(&mut self) -> Dictionary {
        let mut fonts = Dictionary::new();
        for (name, base) in [("F1", "Helvetica"), ("F2", "Helvetica-Bold")] {
            let id = self.doc.add_object(font_dict(base));
            fonts.set(name, Object::Reference(id));
        }
        let mut xobjects = Dictionary::new();
        for (key, image) in &self.images {
            xobjects.set(key.as_str(), Object::Reference(image.id));
        }
        let mut resources = Dictionary::new();
        resources.set("Font", Object::Dictionary(fonts));
        resources.set("XObject", Object::Dictionary(xobjects));
        resources
    }
}

impl Default for Canvas {
    fn default() -> Self {
        Self::new()
    }
}

fn media_box() -> Object {
    Object::Array(vec![
        Object::Integer(0),
        Object::Integer(0),
        Object::Integer(PAGE_WIDTH as i64),
        Object::Integer(PAGE_HEIGHT as i64),
    ])
}

fn font_dict(base: &str) -> Dictionary {
    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"Font".to_vec()));
    dict.set("Subtype", Object::Name(b"Type1".to_vec()));
    dict.set("BaseFont", Object::Name(base.as_bytes().to_vec()));
    dict.set("Encoding", Object::Name(b"WinAnsiEncoding".to_vec()));
    dict
}
