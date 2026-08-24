//! Screenshot montage and collage generator.
//!
//! Combines multiple user-submitted screenshots into a single aesthetic
//! banner image suitable for release cards without sending extra messages.
//! Features an atmospheric multi-image softly blurred background with subtle vignette,
//! adaptive balanced layouts for any aspect ratio mix, rounded screenshot cards,
//! subtle drop shadows, and high-quality Lanczos3 downscaling.

use anyhow::{Context, Result};
use image::imageops::{self, FilterType};
use image::{DynamicImage, GenericImageView, ImageFormat, Rgba, RgbaImage};
use std::io::Cursor;

/// Maximum width for the rendered collage canvas in pixels.
const CANVAS_WIDTH: u32 = 1200;
/// Background padding around the whole canvas in pixels.
const PADDING: u32 = 28;
/// Gap between individual screenshot panels in pixels.
const GAP: u32 = 20;
/// Corner radius for screenshot cards in pixels.
const CORNER_RADIUS: u32 = 16;

/// Layout placement descriptor for a single card.
#[derive(Debug, Clone)]
struct CardPlacement {
    image_idx: usize,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

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

    // Calculate adaptive layout geometry based on image count & aspect ratios
    let (canvas_w, canvas_h, placements) = compute_layout(&decoded_images);

    // 1. Build atmospheric softly-blurred multi-image background
    let mut canvas = generate_ambient_background(&decoded_images, canvas_w, canvas_h);

    // 2. Render each screenshot card with drop shadow, rounded corners, and crisp border
    for p in placements {
        if let Some(img) = decoded_images.get(p.image_idx) {
            let cropped = fill_card(img, p.width, p.height);
            let card = apply_card_styling(&cropped, CORNER_RADIUS);

            // Render soft drop shadow under the card
            draw_card_shadow(&mut canvas, p.x, p.y, p.width, p.height);

            // Alpha-composite the styled card on top
            imageops::overlay(&mut canvas, &card, i64::from(p.x), i64::from(p.y));
        }
    }

    // 3. Convert canvas to DynamicImage and encode to high-quality JPEG
    let mut out = Vec::new();
    let dynamic_img = DynamicImage::ImageRgba8(canvas);
    dynamic_img
        .write_to(&mut Cursor::new(&mut out), ImageFormat::Jpeg)
        .context("Failed to encode collage to JPEG")?;

    Ok(out)
}

/// Computes an adaptive, balanced layout for `images` with no awkward empty grid holes.
fn compute_layout(images: &[DynamicImage]) -> (u32, u32, Vec<CardPlacement>) {
    let n = images.len();
    let available_w = CANVAS_WIDTH.saturating_sub(2 * PADDING);

    let aspects: Vec<f32> = images
        .iter()
        .map(|img| (img.width() as f32) / (img.height().max(1) as f32))
        .collect();

    match n {
        2 => {
            let both_wide = aspects.iter().all(|&a| a >= 1.35);
            if both_wide {
                // Stacked 2 horizontal images
                let avg_aspect = (aspects[0] + aspects[1]) / 2.0;
                let card_w = available_w;
                let card_h = ((card_w as f32 / avg_aspect).round() as u32).clamp(240, 480);
                let canvas_h = 2 * PADDING + 2 * card_h + GAP;

                let placements = vec![
                    CardPlacement {
                        image_idx: 0,
                        x: PADDING,
                        y: PADDING,
                        width: card_w,
                        height: card_h,
                    },
                    CardPlacement {
                        image_idx: 1,
                        x: PADDING,
                        y: PADDING + card_h + GAP,
                        width: card_w,
                        height: card_h,
                    },
                ];
                (CANVAS_WIDTH, canvas_h, placements)
            } else {
                // Side-by-side 2 images
                let card_w = (available_w - GAP) / 2;
                let avg_aspect = (aspects[0] + aspects[1]) / 2.0;
                let card_h = ((card_w as f32 / avg_aspect).round() as u32).clamp(380, 800);
                let canvas_h = 2 * PADDING + card_h;

                let placements = vec![
                    CardPlacement {
                        image_idx: 0,
                        x: PADDING,
                        y: PADDING,
                        width: card_w,
                        height: card_h,
                    },
                    CardPlacement {
                        image_idx: 1,
                        x: PADDING + card_w + GAP,
                        y: PADDING,
                        width: card_w,
                        height: card_h,
                    },
                ];
                (CANVAS_WIDTH, canvas_h, placements)
            }
        }
        3 => {
            let landscape_indices: Vec<usize> = aspects
                .iter()
                .enumerate()
                .filter(|(_, &a)| a >= 1.05)
                .map(|(i, _)| i)
                .collect();

            if landscape_indices.is_empty() {
                // All 3 portrait: 3 clean vertical columns side-by-side
                let card_w = (available_w - 2 * GAP) / 3;
                let avg_aspect = aspects.iter().sum::<f32>() / 3.0;
                let card_h = ((card_w as f32 / avg_aspect).round() as u32).clamp(420, 780);
                let canvas_h = 2 * PADDING + card_h;

                let placements = (0..3)
                    .map(|i| CardPlacement {
                        image_idx: i,
                        x: PADDING + (i as u32) * (card_w + GAP),
                        y: PADDING,
                        width: card_w,
                        height: card_h,
                    })
                    .collect();

                (CANVAS_WIDTH, canvas_h, placements)
            } else if landscape_indices.len() == 1 {
                // 1 landscape + 2 portrait (e.g. 1 hero banner + 2 vertical cards)
                let l_idx = landscape_indices[0];
                let portraits: Vec<usize> = (0..3).filter(|&i| i != l_idx).collect();
                let p1 = portraits[0];
                let p2 = portraits[1];

                // Row 1: Featured landscape banner
                let r1_w = available_w;
                let r1_h = ((r1_w as f32 / aspects[l_idx]).round() as u32).clamp(240, 420);

                // Row 2: Two portrait cards side by side
                let r2_w = (available_w - GAP) / 2;
                let p_aspect = (aspects[p1] + aspects[p2]) / 2.0;
                let r2_h = ((r2_w as f32 / p_aspect).round() as u32).clamp(340, 560);

                let canvas_h = 2 * PADDING + r1_h + GAP + r2_h;

                let placements = vec![
                    CardPlacement {
                        image_idx: l_idx,
                        x: PADDING,
                        y: PADDING,
                        width: r1_w,
                        height: r1_h,
                    },
                    CardPlacement {
                        image_idx: p1,
                        x: PADDING,
                        y: PADDING + r1_h + GAP,
                        width: r2_w,
                        height: r2_h,
                    },
                    CardPlacement {
                        image_idx: p2,
                        x: PADDING + r2_w + GAP,
                        y: PADDING + r1_h + GAP,
                        width: r2_w,
                        height: r2_h,
                    },
                ];

                (CANVAS_WIDTH, canvas_h, placements)
            } else if landscape_indices.len() == 2 {
                // 2 landscape + 1 portrait: Split column layout
                let p_idx = (0..3).find(|&i| !landscape_indices.contains(&i)).unwrap_or(0);
                let l1 = landscape_indices[0];
                let l2 = landscape_indices[1];

                let col1_w = ((available_w - GAP) as f32 * 0.46).round() as u32;
                let col2_w = available_w - GAP - col1_w;

                let total_h = ((col1_w as f32 / aspects[p_idx]).round() as u32).clamp(460, 780);
                let card2_h = (total_h - GAP) / 2;
                let canvas_h = 2 * PADDING + total_h;

                let placements = vec![
                    CardPlacement {
                        image_idx: p_idx,
                        x: PADDING,
                        y: PADDING,
                        width: col1_w,
                        height: total_h,
                    },
                    CardPlacement {
                        image_idx: l1,
                        x: PADDING + col1_w + GAP,
                        y: PADDING,
                        width: col2_w,
                        height: card2_h,
                    },
                    CardPlacement {
                        image_idx: l2,
                        x: PADDING + col1_w + GAP,
                        y: PADDING + card2_h + GAP,
                        width: col2_w,
                        height: card2_h,
                    },
                ];

                (CANVAS_WIDTH, canvas_h, placements)
            } else {
                // All 3 landscape: 1 full width top + 2 side-by-side bottom
                let r1_w = available_w;
                let r1_h = ((r1_w as f32 / aspects[0]).round() as u32).clamp(240, 400);

                let r2_w = (available_w - GAP) / 2;
                let p_aspect = (aspects[1] + aspects[2]) / 2.0;
                let r2_h = ((r2_w as f32 / p_aspect).round() as u32).clamp(220, 380);

                let canvas_h = 2 * PADDING + r1_h + GAP + r2_h;

                let placements = vec![
                    CardPlacement {
                        image_idx: 0,
                        x: PADDING,
                        y: PADDING,
                        width: r1_w,
                        height: r1_h,
                    },
                    CardPlacement {
                        image_idx: 1,
                        x: PADDING,
                        y: PADDING + r1_h + GAP,
                        width: r2_w,
                        height: r2_h,
                    },
                    CardPlacement {
                        image_idx: 2,
                        x: PADDING + r2_w + GAP,
                        y: PADDING + r1_h + GAP,
                        width: r2_w,
                        height: r2_h,
                    },
                ];

                (CANVAS_WIDTH, canvas_h, placements)
            }
        }
        4 => {
            // Symmetrical 2x2 grid
            let card_w = (available_w - GAP) / 2;
            let avg_aspect = aspects.iter().sum::<f32>() / 4.0;
            let card_h = ((card_w as f32 / avg_aspect).round() as u32).clamp(320, 560);
            let canvas_h = 2 * PADDING + 2 * card_h + GAP;

            let placements = vec![
                CardPlacement {
                    image_idx: 0,
                    x: PADDING,
                    y: PADDING,
                    width: card_w,
                    height: card_h,
                },
                CardPlacement {
                    image_idx: 1,
                    x: PADDING + card_w + GAP,
                    y: PADDING,
                    width: card_w,
                    height: card_h,
                },
                CardPlacement {
                    image_idx: 2,
                    x: PADDING,
                    y: PADDING + card_h + GAP,
                    width: card_w,
                    height: card_h,
                },
                CardPlacement {
                    image_idx: 3,
                    x: PADDING + card_w + GAP,
                    y: PADDING + card_h + GAP,
                    width: card_w,
                    height: card_h,
                },
            ];

            (CANVAS_WIDTH, canvas_h, placements)
        }
        _ => {
            // 5+ images: balanced multi-row grid
            let cols = (n as f32).sqrt().ceil() as u32;
            let rows = ((n as f32) / (cols as f32)).ceil() as u32;
            let card_w = (available_w.saturating_sub((cols - 1) * GAP)) / cols;
            let avg_aspect = aspects.iter().sum::<f32>() / (n as f32);
            let card_h = ((card_w as f32 / avg_aspect).round() as u32).clamp(240, 480);
            let canvas_h = 2 * PADDING + rows * card_h + (rows.saturating_sub(1)) * GAP;

            let mut placements = Vec::with_capacity(n);
            for i in 0..n {
                let r = (i as u32) / cols;
                let c = (i as u32) % cols;
                placements.push(CardPlacement {
                    image_idx: i,
                    x: PADDING + c * (card_w + GAP),
                    y: PADDING + r * (card_h + GAP),
                    width: card_w,
                    height: card_h,
                });
            }

            (CANVAS_WIDTH, canvas_h, placements)
        }
    }
}

/// Generates an ambient frosted background by blending ALL input images across the canvas,
/// applying rich gaussian blur and a gentle modern dark vignette tint.
fn generate_ambient_background(images: &[DynamicImage], width: u32, height: u32) -> RgbaImage {
    let small_w = 160u32;
    let small_h = 100u32;
    let mut composite = RgbaImage::new(small_w, small_h);

    // 1. Composite all images horizontally so their colors diffuse across the background
    let slice_w = small_w as f32 / (images.len() as f32);
    for (i, img) in images.iter().enumerate() {
        let x_start = (i as f32 * slice_w).round() as u32;
        let x_end = (((i + 1) as f32 * slice_w).round() as u32).min(small_w);
        let curr_slice_w = x_end.saturating_sub(x_start).max(1);

        let small_slice = img
            .resize_exact(curr_slice_w, small_h, FilterType::Triangle)
            .to_rgba8();
        imageops::overlay(&mut composite, &small_slice, i64::from(x_start), 0);
    }

    // 2. Heavy gaussian blur on the composite so color transitions are silky smooth
    let blurred_small = imageops::blur(&composite, 16.0);

    // 3. Upscale to canvas dimensions with bilinear filtering
    let upscaled = DynamicImage::ImageRgba8(blurred_small)
        .resize_exact(width, height, FilterType::Triangle)
        .to_rgba8();

    // 4. Second smooth blur pass on full canvas
    let mut full_blurred = imageops::blur(&upscaled, 8.0);

    // 5. Apply soft ambient tint + subtle vignette (gently darker at edges, glowing in center)
    let center_x = width as f32 / 2.0;
    let center_y = height as f32 / 2.0;
    let max_dist = (center_x * center_x + center_y * center_y).sqrt().max(1.0);

    for (x, y, pixel) in full_blurred.enumerate_pixels_mut() {
        let dx = x as f32 - center_x;
        let dy = y as f32 - center_y;
        let dist = (dx * dx + dy * dy).sqrt() / max_dist;

        // Base tint alpha: 0.44 in center, 0.68 at far corners
        let tint_alpha = (0.44 + 0.24 * dist).clamp(0.0, 0.90);
        let inv_alpha = 1.0 - tint_alpha;

        let tint_r = 13.0;
        let tint_g = 15.0;
        let tint_b = 22.0;

        let r = ((pixel[0] as f32 * inv_alpha) + (tint_r * tint_alpha)).round() as u8;
        let g = ((pixel[1] as f32 * inv_alpha) + (tint_g * tint_alpha)).round() as u8;
        let b = ((pixel[2] as f32 * inv_alpha) + (tint_b * tint_alpha)).round() as u8;

        *pixel = Rgba([r, g, b, 255]);
    }

    full_blurred
}

/// Scales and fits an image inside (target_w, target_h) with intelligent aspect handling:
/// - If aspect ratios are reasonably matched, crops gently using Lanczos3.
/// - If aspect ratios strongly diverge, uses elegant blurred-letterbox fill so no subject matter is cut off.
fn fill_card(img: &DynamicImage, target_w: u32, target_h: u32) -> DynamicImage {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return img.clone();
    }

    let img_aspect = w as f32 / h as f32;
    let target_aspect = target_w as f32 / target_h as f32;
    let ratio_diff = if img_aspect > target_aspect {
        img_aspect / target_aspect
    } else {
        target_aspect / img_aspect
    };

    if ratio_diff <= 1.35 {
        // Direct Lanczos3 fill with minimal center crop
        let scale_w = target_w as f32 / w as f32;
        let scale_h = target_h as f32 / h as f32;
        let scale = scale_w.max(scale_h);

        let scaled_w = ((w as f32 * scale).round() as u32).max(target_w);
        let scaled_h = ((h as f32 * scale).round() as u32).max(target_h);

        let resized = img.resize_exact(scaled_w, scaled_h, FilterType::Lanczos3);
        let crop_x = (scaled_w.saturating_sub(target_w)) / 2;
        let crop_y = (scaled_h.saturating_sub(target_h)) / 2;

        resized.crop_imm(crop_x, crop_y, target_w, target_h)
    } else {
        // Aspect ratios strongly differ: letterbox with blurred background of the same image
        let bg_fill = img.resize_exact(target_w / 4, target_h / 4, FilterType::Triangle).to_rgba8();
        let bg_blurred = imageops::blur(&bg_fill, 10.0);
        let mut card_bg = DynamicImage::ImageRgba8(bg_blurred)
            .resize_exact(target_w, target_h, FilterType::Triangle)
            .to_rgba8();

        // Darken card background slightly
        for pixel in card_bg.pixels_mut() {
            pixel[0] = ((pixel[0] as f32) * 0.70).round() as u8;
            pixel[1] = ((pixel[1] as f32) * 0.70).round() as u8;
            pixel[2] = ((pixel[2] as f32) * 0.70).round() as u8;
        }

        // Fit main image cleanly inside card
        let scale_w = target_w as f32 / w as f32;
        let scale_h = target_h as f32 / h as f32;
        let scale = scale_w.min(scale_h);

        let fit_w = ((w as f32 * scale).round() as u32).max(1);
        let fit_h = ((h as f32 * scale).round() as u32).max(1);

        let foreground = img.resize_exact(fit_w, fit_h, FilterType::Lanczos3).to_rgba8();

        let ox = (target_w.saturating_sub(fit_w)) / 2;
        let oy = (target_h.saturating_sub(fit_h)) / 2;

        imageops::overlay(&mut card_bg, &foreground, i64::from(ox), i64::from(oy));
        DynamicImage::ImageRgba8(card_bg)
    }
}

/// Applies anti-aliased rounded corners without any harsh outline artifacts.
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
) {
    let shadow_offset_y = 8i32;
    let shadow_spread = 14i32;
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
                let shadow_alpha = (factor * 0.48).clamp(0.0, 1.0);

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
    fn test_three_mixed_images_collage() {
        // 1 vertical, 1 horizontal, 1 vertical (matches user's screenshot)
        let img1 = make_test_image_bytes(300, 450, Rgb([60, 140, 240])); // Blue Portrait
        let img2 = make_test_image_bytes(600, 300, Rgb([240, 180, 60]));  // Yellow Landscape
        let img3 = make_test_image_bytes(300, 450, Rgb([240, 80, 140])); // Pink Portrait

        let result = create_collage(&[img1, img2, img3]).unwrap();
        assert!(!result.is_empty());

        let decoded = image::load_from_memory(&result).unwrap();
        assert_eq!(decoded.width(), CANVAS_WIDTH);
        assert!(decoded.height() > 400);
    }

    #[test]
    fn test_three_vertical_images_collage() {
        let img1 = make_test_image_bytes(300, 600, Rgb([60, 140, 240]));
        let img2 = make_test_image_bytes(300, 600, Rgb([140, 60, 240]));
        let img3 = make_test_image_bytes(300, 600, Rgb([240, 80, 140]));

        let result = create_collage(&[img1, img2, img3]).unwrap();
        assert!(!result.is_empty());

        let decoded = image::load_from_memory(&result).unwrap();
        assert_eq!(decoded.width(), CANVAS_WIDTH);
    }
}
