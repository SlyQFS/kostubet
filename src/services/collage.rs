//! Screenshot montage and collage generator.
//!
//! Combines multiple user-submitted screenshots into a single aesthetic
//! banner image suitable for release cards without sending extra messages.
//! Features an atmospheric softly blurred background, rounded screenshot cards,
//! subtle drop shadows, and high-quality Lanczos3 downscaling.

use anyhow::{Context, Result};
use image::imageops::{self, FilterType};
use image::{DynamicImage, GenericImageView, ImageFormat, Rgba, RgbaImage};
use std::io::Cursor;

/// Maximum width for the rendered collage canvas in pixels.
const CANVAS_MAX_WIDTH: u32 = 1280;
/// Background padding around the whole canvas in pixels.
const PADDING: u32 = 24;
/// Gap between individual screenshot panels in pixels.
const GAP: u32 = 18;
/// Corner radius for screenshot cards in pixels.
const CORNER_RADIUS: u32 = 16;
/// Overlay tint color applied over the blurred background.
const TINT_COLOR: Rgba<u8> = Rgba([16, 17, 22, 218]);
/// Subtle border color around each screenshot card.
const CARD_BORDER_COLOR: Rgba<u8> = Rgba([255, 255, 255, 38]);

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
            if avg_aspect < 0.85 {
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

    let cell_h = (cell_w as f32 * avg_ratio).round().clamp(300.0, 1600.0) as u32;

    let canvas_w = 2 * PADDING + cols * cell_w + (cols.saturating_sub(1)) * GAP;
    let canvas_h = 2 * PADDING + rows * cell_h + (rows.saturating_sub(1)) * GAP;

    // 1. Build atmospheric softly-blurred background
    let mut canvas = generate_blurred_background(&decoded_images, canvas_w, canvas_h);

    // 2. Render each screenshot card with drop shadow, rounded corners, and crisp border
    for (idx, img) in decoded_images.iter().enumerate() {
        let r = (idx as u32) / cols;
        let c = (idx as u32) % cols;

        if r >= rows {
            break;
        }

        let resized = resize_and_crop(img, cell_w, cell_h);
        let card = apply_card_styling(&resized, CORNER_RADIUS);

        let rx = PADDING + c * (cell_w + GAP) + (cell_w.saturating_sub(card.width())) / 2;
        let ry = PADDING + r * (cell_h + GAP) + (cell_h.saturating_sub(card.height())) / 2;

        // Render soft drop shadow under the card
        draw_card_shadow(&mut canvas, rx, ry, card.width(), card.height(), CORNER_RADIUS);

        // Alpha-composite the styled card on top
        imageops::overlay(&mut canvas, &card, i64::from(rx), i64::from(ry));
    }

    // 3. Convert canvas to DynamicImage and encode to high-quality JPEG
    let mut out = Vec::new();
    let dynamic_img = DynamicImage::ImageRgba8(canvas);
    dynamic_img
        .write_to(&mut Cursor::new(&mut out), ImageFormat::Jpeg)
        .context("Failed to encode collage to JPEG")?;

    Ok(out)
}

/// Generates an ambient frosted background by scaling, tiling, softly blurring
/// and tinting the input images.
fn generate_blurred_background(images: &[DynamicImage], width: u32, height: u32) -> RgbaImage {
    let mut bg = RgbaImage::from_pixel(width, height, Rgba([20, 21, 26, 255]));

    if let Some(first) = images.first() {
        // Downscale to a smaller buffer for fast smooth blur
        let small_w = (width / 4).max(64);
        let small_h = (height / 4).max(64);
        let small_bg = first.resize_exact(small_w, small_h, FilterType::Triangle);
        let mut small_rgba = small_bg.to_rgba8();

        // Apply weak-to-medium soft blur
        small_rgba = imageops::blur(&small_rgba, 10.0);

        // Upscale back with bilinear filtering
        let upscaled = DynamicImage::ImageRgba8(small_rgba).resize_exact(width, height, FilterType::Triangle);
        let upscaled_rgba = upscaled.to_rgba8();

        // Overlay with dark translucent tint
        for (x, y, pixel) in bg.enumerate_pixels_mut() {
            let src = upscaled_rgba.get_pixel(x, y);
            let alpha = (TINT_COLOR[3] as f32) / 255.0;
            let inv_alpha = 1.0 - alpha;

            let r = ((src[0] as f32 * inv_alpha) + (TINT_COLOR[0] as f32 * alpha)).round() as u8;
            let g = ((src[1] as f32 * inv_alpha) + (TINT_COLOR[1] as f32 * alpha)).round() as u8;
            let b = ((src[2] as f32 * inv_alpha) + (TINT_COLOR[2] as f32 * alpha)).round() as u8;

            *pixel = Rgba([r, g, b, 255]);
        }
    }

    bg
}

/// Resizes image to fit uniformly inside (target_w, target_h).
fn resize_and_crop(img: &DynamicImage, target_w: u32, target_h: u32) -> DynamicImage {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return img.clone();
    }

    let scale_w = target_w as f32 / w as f32;
    let scale_h = target_h as f32 / h as f32;
    let scale = scale_w.min(scale_h);

    let fit_w = ((w as f32 * scale).round() as u32).max(1);
    let fit_h = ((h as f32 * scale).round() as u32).max(1);

    img.resize_exact(fit_w, fit_h, FilterType::Lanczos3)
}

/// Applies rounded corners and a sleek 1px outer highlight border.
fn apply_card_styling(img: &DynamicImage, radius: u32) -> RgbaImage {
    let (w, h) = img.dimensions();
    let mut rgba = img.to_rgba8();
    let r_f = radius as f32;

    for y in 0..h {
        for x in 0..w {
            let px = rgba.get_pixel_mut(x, y);

            // Compute distance to corner centers
            let dx = if x < radius {
                r_f - (x as f32)
            } else if x >= w.saturating_sub(radius) {
                (x as f32) - (w - radius - 1) as f32
            } else {
                0.0
            };

            let dy = if y < radius {
                r_f - (y as f32)
            } else if y >= h.saturating_sub(radius) {
                (y as f32) - (h - radius - 1) as f32
            } else {
                0.0
            };

            if dx > 0.0 && dy > 0.0 {
                let dist = (dx * dx + dy * dy).sqrt();
                if dist > r_f + 0.5 {
                    // Outside rounded corner
                    px[3] = 0;
                } else if dist > r_f - 0.5 {
                    // Anti-aliased corner edge
                    let alpha = (r_f + 0.5 - dist).clamp(0.0, 1.0);
                    px[3] = ((px[3] as f32) * alpha).round() as u8;
                }
            }

            // Draw 1px subtle highlight border on edges
            let is_edge = x == 0 || y == 0 || x == w - 1 || y == h - 1;
            if is_edge && px[3] > 0 {
                let b_alpha = (CARD_BORDER_COLOR[3] as f32) / 255.0;
                let inv = 1.0 - b_alpha;
                px[0] = ((px[0] as f32 * inv) + (CARD_BORDER_COLOR[0] as f32 * b_alpha)).round() as u8;
                px[1] = ((px[1] as f32 * inv) + (CARD_BORDER_COLOR[1] as f32 * b_alpha)).round() as u8;
                px[2] = ((px[2] as f32 * inv) + (CARD_BORDER_COLOR[2] as f32 * b_alpha)).round() as u8;
            }
        }
    }

    rgba
}

/// Draws a subtle soft drop shadow behind a screenshot card.
fn draw_card_shadow(
    canvas: &mut RgbaImage,
    card_x: u32,
    card_y: u32,
    card_w: u32,
    card_h: u32,
    _radius: u32,
) {
    let shadow_offset_y = 6i32;
    let shadow_spread = 8i32;
    let (cw, ch) = (canvas.width() as i32, canvas.height() as i32);

    let min_x = ((card_x as i32) - shadow_spread).max(0);
    let max_x = ((card_x as i32) + (card_w as i32) + shadow_spread).min(cw - 1);
    let min_y = ((card_y as i32) + shadow_offset_y - shadow_spread).max(0);
    let max_y = ((card_y as i32) + (card_h as i32) + shadow_offset_y + shadow_spread).min(ch - 1);

    let base_x1 = card_x as f32;
    let base_x2 = (card_x + card_w) as f32;
    let base_y1 = (card_y as i32 + shadow_offset_y) as f32;
    let base_y2 = (card_y as i32 + card_h as i32 + shadow_offset_y) as f32;

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let px = x as f32;
            let py = y as f32;

            // Distance to card rectangle
            let dx = if px < base_x1 {
                base_x1 - px
            } else if px > base_x2 {
                px - base_x2
            } else {
                0.0
            };

            let dy = if py < base_y1 {
                base_y1 - py
            } else if py > base_y2 {
                py - base_y2
            } else {
                0.0
            };

            let dist = (dx * dx + dy * dy).sqrt();
            if dist < (shadow_spread as f32) {
                let factor = (1.0 - dist / (shadow_spread as f32)).powi(2);
                let shadow_alpha = (factor * 0.45).clamp(0.0, 1.0);

                let pixel = canvas.get_pixel_mut(x as u32, y as u32);
                let inv = 1.0 - shadow_alpha;
                pixel[0] = ((pixel[0] as f32) * inv).round() as u8;
                pixel[1] = ((pixel[1] as f32) * inv).round() as u8;
                pixel[2] = ((pixel[2] as f32) * inv).round() as u8;
            }
        }
    }
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
