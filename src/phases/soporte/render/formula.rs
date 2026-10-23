//! `Formato_Formula*`: the CPScan formula image on a Letter page, aspect-fit at
//! the top-left corner (25, 26) like the Jasper templates did.

use anyhow::Result;

use super::canvas::{Canvas, Frame};
use super::image::{self, Accept};

/// Width and height of the box the image is fitted into (612 - 2 x 25 - 1, 792 - 26 - 27).
const FIT_WIDTH: f64 = 561.0;
const FIT_HEIGHT: f64 = 739.0;
const LEFT: f64 = 25.0;
const TOP: f64 = 26.0;

/// Renders one Letter page holding `data` (PNG, JPEG, GIF, BMP or TIFF).
pub fn render(data: &[u8]) -> Result<Vec<u8>> {
    let (_, width, height) = image::probe(data, Accept::Any)
        .map_err(|e| anyhow::anyhow!("formulapdf: invalid formula image: {e:#}"))?;
    let scale = (FIT_WIDTH / f64::from(width)).min(FIT_HEIGHT / f64::from(height));
    let mut canvas = Canvas::new();
    canvas
        .image_at(
            data,
            "Im1",
            Accept::Any,
            Frame::new(
                LEFT,
                TOP,
                f64::from(width) * scale,
                f64::from(height) * scale,
            ),
        )
        .map_err(|e| anyhow::anyhow!("formulapdf: import image: {e:#}"))?;
    canvas.end_page();
    canvas.finish()
}
