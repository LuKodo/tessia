//! The patient signature image of the manual support: the Java `GenerarFirma`
//! port. The text follows `nombreParaFirma`, is drawn in an italic font on a
//! transparent PNG and sheared like `AffineTransform.shear`.
//!
//! Deliberate difference (as in Go): Java picked a random font from a network
//! share on every call; here every signature uses the Go Italic font embedded in
//! the binary (`assets/soporte/Go-Italic.ttf`, BSD-style license, see
//! `Go-Italic.LICENSE.txt`), so the output is deterministic.

use std::io::Cursor;
use std::sync::LazyLock;

use ab_glyph::{Font, FontRef, Glyph, PxScale, ScaleFont, point};
use anyhow::{Result, anyhow, bail};
use image::{ImageBuffer, ImageFormat, Rgba, RgbaImage};

static FONT: LazyLock<std::result::Result<FontRef<'static>, String>> = LazyLock::new(|| {
    FontRef::try_from_slice(include_bytes!("../../../../assets/soporte/Go-Italic.ttf"))
        .map_err(|e| e.to_string())
});

const START_SIZE: f64 = 120.0;
const MIN_SIZE: f64 = 40.0;
const SIZE_STEP: f64 = 5.0;
const MAX_WIDTH: f64 = 1000.0;
const MARGIN: u32 = 60;
const SHEAR: f64 = -0.15;
/// Replaces an empty patient name, so a signature is never blank.
pub const NO_NAME: &str = "SIN_NOMBRE";

/// Go `nombreParaFirma`: names of up to two words are kept whole; longer names
/// keep the first name and the surname(s) at the positions Java chose, adding
/// the next word when the surname is shorter than three letters (`DE`, `DEL`).
pub fn signature_text(name: &str) -> String {
    let w: Vec<&str> = name.split_whitespace().collect();
    let short = |i: usize| w[i].chars().count() < 3;
    let pick = |i: usize| {
        if short(i) {
            format!("{} {} {}", w[0], w[i], w[i + 1])
        } else {
            format!("{} {}", w[0], w[i])
        }
    };
    match w.len() {
        0 => NO_NAME.to_owned(),
        1 | 2 => w.join(" "),
        3 => pick(1),
        4 => pick(2),
        5 => pick(3),
        n => format!("{} {} {}", w[0], w[n - 2], w[n - 1]),
    }
}

/// Drops control characters, which have no glyph.
fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

fn text_width(font: &impl Font, scale: PxScale, text: &str) -> f32 {
    let scaled = font.as_scaled(scale);
    let mut width = 0.0;
    let mut previous = None;
    for c in text.chars() {
        let id = scaled.glyph_id(c);
        if let Some(prev) = previous {
            width += scaled.kern(prev, id);
        }
        width += scaled.h_advance(id);
        previous = Some(id);
    }
    width
}

/// Draws the signature of `name` as a PNG with a transparent background: black
/// text, 120 pt shrunk by 5 pt steps (never below 40 pt) until it is at most
/// 1000 px wide, sheared by -0.15, on an image (text width + 60) x (2 x size)
/// with the baseline at `height / 2 + size / 3`.
pub fn render(name: &str) -> Result<Vec<u8>> {
    let font = FONT.as_ref().map_err(|e| anyhow!("signature font: {e}"))?;
    let mut text = clean(&signature_text(name));
    if text.trim().is_empty() {
        text = NO_NAME.to_owned();
    }
    let scale_of = |size: f64| {
        font.pt_to_px_scale(size as f32)
            .unwrap_or(PxScale::from(size as f32))
    };
    let mut size = START_SIZE;
    let mut width = text_width(font, scale_of(size), &text);
    while f64::from(width) > MAX_WIDTH && size > MIN_SIZE {
        size -= SIZE_STEP;
        width = text_width(font, scale_of(size), &text);
    }
    let (w, h) = (width as u32 + MARGIN, (size * 2.0) as u32);
    if w <= MARGIN || h == 0 {
        bail!("signature: empty text");
    }
    let baseline = h / 2 + (size / 3.0) as u32;
    let flat = draw_flat(font, scale_of(size), &text, w, h, baseline);
    encode(&shear(&flat, baseline))
}

/// Black text on a transparent image: the coverage is the alpha channel.
fn draw_flat(
    font: &FontRef<'_>,
    scale: PxScale,
    text: &str,
    w: u32,
    h: u32,
    baseline: u32,
) -> RgbaImage {
    let mut img: RgbaImage = ImageBuffer::from_pixel(w, h, Rgba([0, 0, 0, 0]));
    let scaled = font.as_scaled(scale);
    let mut x = (MARGIN / 2) as f32;
    let mut previous = None;
    for c in text.chars() {
        let id = scaled.glyph_id(c);
        if let Some(prev) = previous {
            x += scaled.kern(prev, id);
        }
        let glyph: Glyph = id.with_scale_and_position(scale, point(x, baseline as f32));
        x += scaled.h_advance(id);
        previous = Some(id);
        let Some(outlined) = font.outline_glyph(glyph) else {
            continue;
        };
        let bounds = outlined.px_bounds();
        outlined.draw(|gx, gy, coverage| {
            let (px, py) = (
                bounds.min.x as i64 + i64::from(gx),
                bounds.min.y as i64 + i64::from(gy),
            );
            if px < 0 || py < 0 || px >= i64::from(w) || py >= i64::from(h) {
                return;
            }
            let pixel = img.get_pixel_mut(px as u32, py as u32);
            let alpha = (f32::from(pixel.0[3]) / 255.0
                + coverage * (1.0 - f32::from(pixel.0[3]) / 255.0))
                .min(1.0);
            pixel.0[3] = (alpha * 255.0).round() as u8;
        });
    }
    img
}

/// `x' = x + shear * (y - baseline)`: points above the baseline lean right
/// (bilinear sampling of the alpha channel, pixel centers).
fn shear(src: &RgbaImage, baseline: u32) -> RgbaImage {
    let (w, h) = src.dimensions();
    let mut out: RgbaImage = ImageBuffer::from_pixel(w, h, Rgba([0, 0, 0, 0]));
    let alpha = |x: i64, y: i64| -> f64 {
        if x < 0 || y < 0 || x >= i64::from(w) || y >= i64::from(h) {
            0.0
        } else {
            f64::from(src.get_pixel(x as u32, y as u32).0[3])
        }
    };
    for y in 0..h {
        for x in 0..w {
            let (cx, cy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            let sx = cx - SHEAR * (cy - f64::from(baseline)) - 0.5;
            let (x0, fx) = (sx.floor(), sx - sx.floor());
            let (x0, y0) = (x0 as i64, i64::from(y));
            let value = alpha(x0, y0) * (1.0 - fx) + alpha(x0 + 1, y0) * fx;
            out.put_pixel(x, y, Rgba([0, 0, 0, value.round() as u8]));
        }
    }
    out
}

fn encode(img: &RgbaImage) -> Result<Vec<u8>> {
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png)?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(png: &[u8]) -> RgbaImage {
        image::load_from_memory(png).expect("png").to_rgba8()
    }

    #[test]
    fn the_signature_text_follows_nombre_para_firma() {
        assert_eq!(signature_text(""), "SIN_NOMBRE");
        assert_eq!(signature_text("ANA"), "ANA");
        assert_eq!(signature_text("ANA PEREZ"), "ANA PEREZ");
        assert_eq!(signature_text("ANA MARIA PEREZ"), "ANA MARIA");
        assert_eq!(signature_text("ANA DE LA CRUZ"), "ANA LA CRUZ");
        assert_eq!(signature_text("ANA MARIA PEREZ GOMEZ"), "ANA PEREZ");
        assert_eq!(signature_text("ANA MARIA PEREZ DEL RIO"), "ANA DEL");
        assert_eq!(signature_text("ANA MARIA PEREZ DE RIO"), "ANA DE RIO");
        assert_eq!(signature_text("A B C D E F"), "A E F");
    }

    #[test]
    fn the_image_is_a_transparent_png_with_ink_and_the_documented_size() {
        let img = decode(&render("ANA PEREZ").expect("render"));
        let (w, h) = img.dimensions();
        assert_eq!(h, 240, "2 x 120 pt");
        assert!(w > 60 + 100 && w <= 1060, "{w}");
        assert!(img.pixels().any(|p| p.0[3] == 0), "transparent background");
        assert!(img.pixels().any(|p| p.0[3] > 200), "opaque ink");
        assert!(img.pixels().all(|p| p.0[..3] == [0, 0, 0]), "black ink");
    }

    #[test]
    fn long_names_shrink_the_font_but_never_below_40_pt() {
        let img = decode(&render("WWWWWWWWWWWWWWWWWWWW WWWWWWWWWWWWWWWWWWWW").expect("render"));
        assert!(img.height() >= 80 && img.height() < 240, "{}", img.height());
        assert!(img.width() <= 1000 + 60 + 1 || img.height() == 80);
    }

    #[test]
    fn the_output_is_deterministic_and_never_blank() {
        assert_eq!(render("LUIS").expect("a"), render("LUIS").expect("b"));
        let blank = decode(&render("   ").expect("render"));
        assert!(blank.pixels().any(|p| p.0[3] > 0), "SIN_NOMBRE is drawn");
        let controls = decode(&render("\u{7}\u{8}").expect("render"));
        assert!(controls.pixels().any(|p| p.0[3] > 0));
    }

    #[test]
    fn the_ink_leans_right_above_the_baseline() {
        let img = decode(&render("IIII").expect("render"));
        let centroid = |rows: std::ops::Range<u32>| {
            let (mut sum, mut n) = (0.0, 0.0);
            for y in rows {
                for x in 0..img.width() {
                    let a = f64::from(img.get_pixel(x, y).0[3]);
                    sum += a * f64::from(x);
                    n += a;
                }
            }
            sum / n
        };
        let top = centroid(70..110);
        let bottom = centroid(130..160);
        assert!(top > bottom, "top {top} bottom {bottom}");
    }
}
