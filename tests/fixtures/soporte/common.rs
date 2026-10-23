//! Synthetic inputs shared by the soporte integration tests.
#![allow(dead_code)]

use std::io::Cursor;

use image::{DynamicImage, ImageBuffer, ImageFormat, Luma, Rgb, Rgba};
use lopdf::content::Content;
use lopdf::{Document, Object};
use tessia::phases::soporte::render::canvas::{Align, Canvas, Frame};

pub fn encode(img: DynamicImage, format: ImageFormat) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, format).expect("encode");
    out.into_inner()
}

pub fn rgb_image(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
    let buf = ImageBuffer::from_fn(width, height, |x, y| {
        Rgb([(x % 256) as u8, (y % 256) as u8, 40])
    });
    encode(DynamicImage::ImageRgb8(buf), format)
}

pub fn gray_image(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
    let buf = ImageBuffer::from_fn(width, height, |x, _| Luma([(x % 256) as u8]));
    encode(DynamicImage::ImageLuma8(buf), format)
}

pub fn rgba_image(width: u32, height: u32) -> Vec<u8> {
    let buf = ImageBuffer::from_fn(width, height, |x, _| Rgba([10, 20, 30, (x % 256) as u8]));
    encode(DynamicImage::ImageRgba8(buf), ImageFormat::Png)
}

/// A PDF of `pages` Letter pages whose n-th page says `"<label> page n"`.
pub fn text_pdf(label: &str, pages: usize) -> Vec<u8> {
    let mut canvas = Canvas::new();
    for n in 1..=pages {
        canvas.text(
            &format!("{label} page {n}"),
            Frame::new(50.0, 50.0, 400.0, 20.0),
            12.0,
            65.0,
            false,
            Align::Left,
        );
        canvas.end_page();
    }
    canvas.finish().expect("pdf")
}

/// Operators of every page: `(operator, operands)` flattened per page.
pub fn page_operations(pdf: &[u8]) -> Vec<Vec<(String, Vec<Object>)>> {
    let doc = Document::load_mem(pdf).expect("load");
    doc.get_pages()
        .into_values()
        .map(|id| {
            Content::decode(&doc.get_page_content(id))
                .expect("content")
                .operations
                .into_iter()
                .map(|op| (op.operator, op.operands))
                .collect()
        })
        .collect()
}

pub fn number(object: &Object) -> f64 {
    f64::from(object.as_float().expect("number"))
}
