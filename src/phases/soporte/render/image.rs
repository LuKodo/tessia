//! Decoding of formula, logo and signature images and their embedding as PDF
//! image XObjects.
//!
//! * PNG, GIF, BMP and TIFF are decoded to 8-bit pixels and stored Flate
//!   compressed (an alpha channel becomes an `SMask`); GIF/BMP/TIFF are thereby
//!   "normalized to PNG" in the sense of Go: a single lossless frame.
//! * A JPEG is kept as is (`DCTDecode`) when it is a plain 8-bit gray or RGB
//!   one; other JPEGs (CMYK, Adobe transform 0) are re-encoded as pixels.
//! * EXIF orientation is NOT applied (the observed Java behavior).
//! * The file is always fully decoded once, so a truncated payload is an error.

use std::io::Cursor;

use anyhow::{Context, Result, bail};
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

/// Decoded images larger than this are refused (memory guard: 4 bytes per pixel
/// while decoding, with up to 35 invoices in flight).
pub const MAX_PIXELS: u64 = 40_000_000;

/// Which formats a caller accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accept {
    /// PNG, JPEG, GIF, BMP and TIFF (formula images).
    Any,
    /// PNG and JPEG (logo and signature).
    PngJpeg,
}

/// An image embedded in a document.
#[derive(Debug, Clone, Copy)]
pub struct Embedded {
    pub id: ObjectId,
    pub width: u32,
    pub height: u32,
}

/// Pixel size of an image without decoding it, validating format and limits.
pub fn probe(data: &[u8], accept: Accept) -> Result<(ImageFormat, u32, u32)> {
    probe_limited(data, accept, MAX_PIXELS)
}

fn probe_limited(data: &[u8], accept: Accept, max_pixels: u64) -> Result<(ImageFormat, u32, u32)> {
    let format = image::guess_format(data).context("formato de imagen no reconocido")?;
    let allowed = match accept {
        Accept::Any => matches!(
            format,
            ImageFormat::Png
                | ImageFormat::Jpeg
                | ImageFormat::Gif
                | ImageFormat::Bmp
                | ImageFormat::Tiff
        ),
        Accept::PngJpeg => matches!(format, ImageFormat::Png | ImageFormat::Jpeg),
    };
    if !allowed {
        bail!("unsupported image format {format:?}");
    }
    let decoder = ImageReader::with_format(Cursor::new(data), format)
        .into_decoder()
        .context("imagen invalida")?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 {
        bail!("invalid image dimensions");
    }
    if u64::from(width) * u64::from(height) > max_pixels {
        bail!("image dimensions exceed resource limits");
    }
    Ok((format, width, height))
}

/// Decodes `data` completely (errors on truncated files).
fn decode(data: &[u8], format: ImageFormat) -> Result<DynamicImage> {
    let mut reader = ImageReader::with_format(Cursor::new(data), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_PIXELS * 4 + (64 << 20));
    reader.limits(limits);
    reader.decode().context("decode image")
}

/// Embeds `data` as an image XObject of `doc`.
pub fn embed(doc: &mut Document, data: &[u8], accept: Accept) -> Result<Embedded> {
    let (format, width, height) = probe(data, accept)?;
    let decoded = decode(data, format)?;
    let plain = (format == ImageFormat::Jpeg)
        .then(|| plain_jpeg_components(data))
        .flatten();
    if let Some(components) = plain {
        let id = add_jpeg(doc, data, width, height, components);
        return Ok(Embedded { id, width, height });
    }
    let id = add_pixels(doc, &decoded, width, height)?;
    Ok(Embedded { id, width, height })
}

fn image_dict(width: u32, height: u32, gray: bool) -> Dictionary {
    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"XObject".to_vec()));
    dict.set("Subtype", Object::Name(b"Image".to_vec()));
    dict.set("Width", Object::Integer(i64::from(width)));
    dict.set("Height", Object::Integer(i64::from(height)));
    dict.set(
        "ColorSpace",
        Object::Name(if gray {
            b"DeviceGray".to_vec()
        } else {
            b"DeviceRGB".to_vec()
        }),
    );
    dict.set("BitsPerComponent", Object::Integer(8));
    dict
}

fn add_jpeg(doc: &mut Document, data: &[u8], width: u32, height: u32, components: u8) -> ObjectId {
    let mut dict = image_dict(width, height, components == 1);
    dict.set("Filter", Object::Name(b"DCTDecode".to_vec()));
    doc.add_object(Stream::new(dict, data.to_vec()).with_compression(false))
}

fn add_pixels(doc: &mut Document, img: &DynamicImage, width: u32, height: u32) -> Result<ObjectId> {
    let color = img.color();
    let gray = !color.has_color();
    let samples = if gray {
        img.to_luma8().into_raw()
    } else {
        img.to_rgb8().into_raw()
    };
    let mut dict = image_dict(width, height, gray);
    if color.has_alpha() {
        let alpha: Vec<u8> = img.to_rgba8().pixels().map(|p| p.0[3]).collect();
        if alpha.iter().any(|a| *a != 255) {
            let mut mask = image_dict(width, height, true);
            mask.set(
                "Decode",
                Object::Array(vec![Object::Integer(0), Object::Integer(1)]),
            );
            let id = add_flate(doc, mask, alpha)?;
            dict.set("SMask", Object::Reference(id));
        }
    }
    add_flate(doc, dict, samples)
}

fn add_flate(doc: &mut Document, dict: Dictionary, data: Vec<u8>) -> Result<ObjectId> {
    let mut stream = Stream::new(dict, data);
    stream.compress().context("compress image")?;
    Ok(doc.add_object(stream))
}

/// Number of components of a baseline/progressive 8-bit JPEG that a PDF reader
/// decodes without extra parameters (1 = gray, 3 = YCbCr/RGB); `None` for the
/// rest (CMYK/YCCK, Adobe transform 0, 12-bit).
fn plain_jpeg_components(data: &[u8]) -> Option<u8> {
    let (mut pos, mut components, mut adobe_transform) = (2usize, None, None);
    while pos + 4 <= data.len() {
        if data[pos] != 0xFF {
            pos += 1;
            continue;
        }
        let marker = data[pos + 1];
        if marker == 0xFF || marker == 0x00 || (0xD0..=0xD9).contains(&marker) || marker == 0x01 {
            pos += if marker == 0xFF { 1 } else { 2 };
            continue;
        }
        let len = usize::from(u16::from_be_bytes([data[pos + 2], data[pos + 3]]));
        let body = data.get(pos + 4..pos + 2 + len)?;
        match marker {
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                if body.first() != Some(&8) {
                    return None;
                }
                components = body.get(5).copied();
            }
            0xEE if body.starts_with(b"Adobe") => adobe_transform = body.get(11).copied(),
            0xDA => break,
            _ => {}
        }
        pos += 2 + len;
    }
    match (components?, adobe_transform) {
        (1, _) => Some(1),
        (3, Some(0)) => None,
        (3, _) => Some(3),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use image::{ImageBuffer, Luma, Rgb, Rgba};

    use super::*;

    pub fn encode(img: DynamicImage, format: ImageFormat) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, format).expect("encode");
        out.into_inner()
    }

    pub fn rgb(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
        let buf = ImageBuffer::from_fn(width, height, |x, y| {
            Rgb([(x % 256) as u8, (y % 256) as u8, 90])
        });
        encode(DynamicImage::ImageRgb8(buf), format)
    }

    pub fn gray(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
        let buf = ImageBuffer::from_fn(width, height, |x, _| Luma([(x % 256) as u8]));
        encode(DynamicImage::ImageLuma8(buf), format)
    }

    pub fn rgba(width: u32, height: u32) -> Vec<u8> {
        let buf = ImageBuffer::from_fn(width, height, |x, _| Rgba([0, 0, 0, (x % 256) as u8]));
        encode(DynamicImage::ImageRgba8(buf), ImageFormat::Png)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    fn xobject(doc: &Document, id: ObjectId) -> &Stream {
        match doc.get_object(id) {
            Ok(Object::Stream(stream)) => stream,
            other => panic!("not a stream: {other:?}"),
        }
    }

    fn filter(stream: &Stream) -> Vec<u8> {
        match stream.dict.get(b"Filter") {
            Ok(Object::Name(name)) => name.clone(),
            other => panic!("no filter: {other:?}"),
        }
    }

    #[test]
    fn a_plain_jpeg_is_kept_as_dct() {
        let data = rgb(20, 10, ImageFormat::Jpeg);
        let mut doc = Document::with_version("1.5");
        let image = embed(&mut doc, &data, Accept::Any).expect("embed");
        assert_eq!((image.width, image.height), (20, 10));
        let stream = xobject(&doc, image.id);
        assert_eq!(filter(stream), b"DCTDecode");
        assert_eq!(stream.content, data);
    }

    #[test]
    fn a_gray_jpeg_uses_the_gray_color_space() {
        let data = gray(8, 8, ImageFormat::Jpeg);
        let mut doc = Document::with_version("1.5");
        let image = embed(&mut doc, &data, Accept::PngJpeg).expect("embed");
        let stream = xobject(&doc, image.id);
        assert_eq!(
            stream.dict.get(b"ColorSpace").ok(),
            Some(&Object::Name(b"DeviceGray".to_vec()))
        );
    }

    #[test]
    fn gif_bmp_and_tiff_become_flate_pixels() {
        for format in [ImageFormat::Gif, ImageFormat::Bmp, ImageFormat::Tiff] {
            let data = rgb(12, 7, format);
            let mut doc = Document::with_version("1.5");
            let image = embed(&mut doc, &data, Accept::Any).expect("embed");
            assert_eq!((image.width, image.height), (12, 7), "{format:?}");
            let stream = xobject(&doc, image.id);
            assert_eq!(filter(stream), b"FlateDecode");
            assert_eq!(
                stream.decompressed_content().expect("inflate").len(),
                12 * 7 * 3
            );
        }
    }

    #[test]
    fn png_alpha_becomes_a_soft_mask_and_opaque_png_does_not() {
        let mut doc = Document::with_version("1.5");
        let image = embed(&mut doc, &rgba(16, 4), Accept::PngJpeg).expect("embed");
        let stream = xobject(&doc, image.id);
        let Ok(Object::Reference(mask)) = stream.dict.get(b"SMask") else {
            panic!("no soft mask");
        };
        assert_eq!(
            xobject(&doc, *mask)
                .decompressed_content()
                .expect("inflate")
                .len(),
            16 * 4
        );
        let opaque = embed(&mut doc, &rgb(4, 4, ImageFormat::Png), Accept::PngJpeg).expect("embed");
        assert!(!xobject(&doc, opaque.id).dict.has(b"SMask"));
    }

    #[test]
    fn delivery_images_reject_formats_other_than_png_and_jpeg() {
        let mut doc = Document::with_version("1.5");
        let err = embed(&mut doc, &rgb(4, 4, ImageFormat::Gif), Accept::PngJpeg).expect_err("gif");
        assert!(
            err.to_string().contains("unsupported image format"),
            "{err:#}"
        );
    }

    #[test]
    fn garbage_and_truncated_files_are_errors() {
        let mut doc = Document::with_version("1.5");
        assert!(embed(&mut doc, b"not an image at all", Accept::Any).is_err());
        let mut png = rgb(64, 64, ImageFormat::Png);
        png.truncate(png.len() / 2);
        assert!(embed(&mut doc, &png, Accept::Any).is_err());
    }

    #[test]
    fn oversized_dimensions_are_refused_before_decoding() {
        let png = rgb(20, 20, ImageFormat::Png);
        assert!(probe_limited(&png, Accept::Any, 400).is_ok());
        let err = probe_limited(&png, Accept::Any, 399).expect_err("too large");
        assert!(err.to_string().contains("exceed"), "{err:#}");
    }
}
