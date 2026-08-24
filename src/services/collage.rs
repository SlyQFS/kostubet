//! Screenshot montage and collage generator.
//!
//! Combines multiple user-submitted screenshots into a single aesthetic
//! banner image suitable for release cards without sending extra messages.

use anyhow::{Context, Result};
use image::imageops::{self, FilterType};
use image::{DynamicImage, GenericImageView, ImageFormat, Rgba, RgbaImage};
use std::io::Cursor;

/// Maximum width for the rendered collage canvas in pixels.
const CANVAS_MAX_WIDTH: u32 = 1280;
/// Background padding around the whole canvas in pixels.
const PADDING: u32 = 16;
/// Gap between individual screenshot panels in pixels.
const GAP: u32 = 12;
/// Background color: deep dark tone (#181818).
const BG_COLOR: Rgba<u8> = Rgba([24, 24, 24, 255]);

/// Creates a unified JPEG collage from a list of raw image byte buffers.
/// If only one image is supplied, returns the original bytes unmodified.
pub fn create_collage(images_raw: &[Vec<u8>]) -> Result<Vec<u8>> {
    if images_raw.is_empty() {
        return Err(anyhow::anyhow!("No images provided for collage"));
    }

    if images_raw.len() == 1 {
        return Ok(images_raw[0].clone());
    }

    // Decode all images
    let mut decoded_images = Vec::with_capacity(images_raw.len());
    for raw in images_raw {
        if let Ok(img) = image::load_from_memory(raw) {
            decoded_images.push(img);
        }
    }

    if decoded_images.is_empty() {
        return Err(anyhow::anyhow!("Failed to decode any images for collage"));
    }

    if decoded_images.len() == 1 {
        return Ok(images_raw[0].clone());
    }

    let n = decoded_images.len();

    // Determine grid arrangement (columns, rows)
    let (cols, rows) = match n {
        2 => {
            let avg_aspect: f32 = decoded_images
                .iter()
                .map(|img| img.width() as f32 / img.height().max(1) as f32)
                .sum::<f32>()
                / 2.0;
            if avg_aspect < 1.0 {
                (2, 1) // 2 vertical screenshots side by side
            } else {
                (1, 2) // 2 horizontal screenshots stacked
            }
        }
        3 => {
            let avg_aspect: f32 = decoded_images
                .iter()
                .map(|img| img.width() as f32 / img.height().max(1) as f32)
                .sum::<f32>()
                / 3.0;
            if avg_aspect < 0.8 {
                (3, 1) // 3 vertical phone screens side by side
            } else {
                (2, 2)
            }
        }
        4 => (2, 2),
        5 | 6 => (3, 2),
        _ => {
            let cols = (n as f32).sqrt().ceil() as u32;
            let rows = ((n as f32) / (cols as f32)).ceil() as u32;
            (cols, rows)
        }
    };

    // Calculate cell dimensions
    let total_gap_x = (cols.saturating_sub(1)) * GAP;
    let available_w = CANVAS_MAX_WIDTH.saturating_sub(2 * PADDING + total_gap_x);
    let cell_w = available_w / cols;

    // Target cell height based on average aspect ratio of the inputs
    let avg_ratio = decoded_images
        .iter()
        .map(|img| img.height() as f32 / img.width().max(1) as f32)
        .sum::<f32>()
        / (n as f32);

    let cell_h = (cell_w as f32 * avg_ratio).round().min(1600.0) as u32;

    let canvas_w = 2 * PADDING + cols * cell_w + (cols.saturating_sub(1)) * GAP;
    let canvas_h = 2 * PADDING + rows * cell_h + (rows.saturating_sub(1)) * GAP;

    let mut canvas = RgbaImage::from_pixel(canvas_w, canvas_h, BG_COLOR);

    for (idx, img) in decoded_images.iter().enumerate() {
        let r = (idx as u32) / cols;
        let c = (idx as u32) % cols;

        if r >= rows {
            break;
        }

        // Resize image to fit nicely inside (cell_w, cell_h)
        let resized = resize_to_fit(img, cell_w, cell_h);
        let rx = PADDING + c * (cell_w + GAP) + (cell_w.saturating_sub(resized.width())) / 2;
        let ry = PADDING + r * (cell_h + GAP) + (cell_h.saturating_sub(resized.height())) / 2;

        imageops::overlay(&mut canvas, &resized, i64::from(rx), i64::from(ry));
    }

    // Convert canvas to DynamicImage and encode to JPEG
    let mut out = Vec::new();
    let dynamic_img = DynamicImage::ImageRgba8(canvas);
    dynamic_img
        .write_to(&mut Cursor::new(&mut out), ImageFormat::Jpeg)
        .context("Failed to encode collage to JPEG")?;

    Ok(out)
}

fn resize_to_fit(img: &DynamicImage, max_w: u32, max_h: u32) -> DynamicImage {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return img.clone();
    }

    let scale_w = max_w as f32 / w as f32;
    let scale_h = max_h as f32 / h as f32;
    let scale = scale_w.min(scale_h);

    let target_w = ((w as f32 * scale).round() as u32).max(1);
    let target_h = ((h as f32 * scale).round() as u32).max(1);

    img.resize(target_w, target_h, FilterType::Triangle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn make_test_image_bytes(w: u32, h: u32, color: Rgb<u8>) -> Vec<u8> {
        let img = RgbImage::from_pixel(w, h, color);
        let dyn_img = DynamicImage::ImageRgb8(img);
        let mut out = Vec::new();
        dyn_img
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn test_single_image_passthrough() {
        let raw = make_test_image_bytes(100, 200, Rgb([255, 0, 0]));
        let result = create_collage(std::slice::from_ref(&raw)).unwrap();
        assert_eq!(result, raw);
    }

    #[test]
    fn test_two_vertical_images_collage() {
        let img1 = make_test_image_bytes(100, 200, Rgb([255, 0, 0]));
        let img2 = make_test_image_bytes(100, 200, Rgb([0, 255, 0]));

        let result = create_collage(&[img1, img2]).unwrap();
        assert!(!result.is_empty());

        let decoded = image::load_from_memory(&result).unwrap();
        assert!(decoded.width() > 100);
        assert!(decoded.height() > 100);
    }

    #[test]
    fn test_four_images_collage() {
        let img1 = make_test_image_bytes(100, 200, Rgb([255, 0, 0]));
        let img2 = make_test_image_bytes(100, 200, Rgb([0, 255, 0]));
        let img3 = make_test_image_bytes(100, 200, Rgb([0, 0, 255]));
        let img4 = make_test_image_bytes(100, 200, Rgb([255, 255, 0]));

        let result = create_collage(&[img1, img2, img3, img4]).unwrap();
        assert!(!result.is_empty());

        let decoded = image::load_from_memory(&result).unwrap();
        assert!(decoded.width() > 200);
    }

    #[test]
    fn test_three_vertical_images_collage() {
        let img1 = make_test_image_bytes(100, 200, Rgb([255, 0, 0]));
        let img2 = make_test_image_bytes(100, 200, Rgb([0, 255, 0]));
        let img3 = make_test_image_bytes(100, 200, Rgb([0, 0, 255]));

        let result = create_collage(&[img1, img2, img3]).unwrap();
        assert!(!result.is_empty());

        let decoded = image::load_from_memory(&result).unwrap();
        assert!(decoded.width() > 100);
        assert!(decoded.height() > 100);
    }
}
