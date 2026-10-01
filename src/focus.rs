use eframe::egui;
use image::GrayImage;

/// Each cell aggregates gradients computed at native source resolution.
/// Values use a fixed scale across images, never per-image normalization.
#[derive(Clone)]
pub struct FocusMap {
    pub width: usize,
    pub height: usize,
    pub source_size: [usize; 2],
    pub energy: Vec<u8>,
}
impl FocusMap {
    pub fn compute(gray: &GrayImage) -> Self {
        const BLOCK: usize = 4;
        let (sw, sh) = (gray.width() as usize, gray.height() as usize);
        let (width, height) = (sw.div_ceil(BLOCK), sh.div_ceil(BLOCK));
        let mut energy = vec![0; width * height];
        let pixels = gray.as_raw();
        if sw >= 3 && sh >= 3 {
            for cy in 0..height {
                for cx in 0..width {
                    let mut sum = 0.0_f32;
                    let mut samples = 0;
                    let mut support = 0;
                    for y in (cy * BLOCK).max(1)..((cy + 1) * BLOCK).min(sh - 1) {
                        for x in (cx * BLOCK).max(1)..((cx + 1) * BLOCK).min(sw - 1) {
                            let at = |dx: isize, dy: isize| {
                                pixels
                                    [y.saturating_add_signed(dy) * sw + x.saturating_add_signed(dx)]
                                    as f32
                            };
                            let gx = at(1, -1) + 2.0 * at(1, 0) + at(1, 1)
                                - at(-1, -1)
                                - 2.0 * at(-1, 0)
                                - at(-1, 1);
                            let gy = at(-1, 1) + 2.0 * at(0, 1) + at(1, 1)
                                - at(-1, -1)
                                - 2.0 * at(0, -1)
                                - at(1, -1);
                            let magnitude = (gx * gx + gy * gy).sqrt() / 1020.0;
                            // Fixed floor rejects low-amplitude noise; require spatial support.
                            if magnitude > 0.03 {
                                sum += magnitude - 0.03;
                                support += 1;
                            }
                            samples += 1;
                        }
                    }
                    if samples > 0 && support >= 3 {
                        energy[cy * width + cx] = (sum / samples as f32 * 255.0).min(255.0) as u8;
                    }
                }
            }
        }
        Self {
            width,
            height,
            source_size: [sw, sh],
            energy,
        }
    }
    pub fn score(&self, region: egui::Rect) -> f32 {
        let min = region.min.max(egui::Pos2::ZERO);
        let max = region.max.min(egui::pos2(
            self.source_size[0] as f32,
            self.source_size[1] as f32,
        ));
        let x0 = (min.x / self.source_size[0].max(1) as f32 * self.width as f32).floor() as usize;
        let y0 = (min.y / self.source_size[1].max(1) as f32 * self.height as f32).floor() as usize;
        let x1 = (max.x / self.source_size[0].max(1) as f32 * self.width as f32).ceil() as usize;
        let y1 = (max.y / self.source_size[1].max(1) as f32 * self.height as f32).ceil() as usize;
        let mut sum = 0_u64;
        let mut count = 0;
        for y in y0.min(self.height)..y1.min(self.height) {
            for x in x0.min(self.width)..x1.min(self.width) {
                sum += self.energy[y * self.width + x] as u64;
                count += 1;
            }
        }
        if count == 0 {
            0.0
        } else {
            sum as f32 / count as f32
        }
    }
    pub fn overlay(&self, threshold: u8, opacity: u8) -> egui::ColorImage {
        let pixels = self
            .energy
            .iter()
            .map(|&value| {
                if value >= threshold {
                    egui::Color32::from_rgba_unmultiplied(40, 255, 120, opacity)
                } else {
                    egui::Color32::TRANSPARENT
                }
            })
            .collect();
        egui::ColorImage::new([self.width, self.height], pixels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn low_amplitude_noise_is_suppressed_and_texture_scores_are_repeatable() {
        let region = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(128.0, 128.0));
        let noise = GrayImage::from_fn(128, 128, |x, y| {
            image::Luma([98 + ((x.wrapping_mul(1664525) ^ y.wrapping_mul(1013904223)) % 5) as u8])
        });
        assert_eq!(FocusMap::compute(&noise).score(region), 0.0);
        let texture = GrayImage::from_fn(128, 128, |x, y| {
            image::Luma([if (x / 5 + y / 7) % 2 == 0 { 220 } else { 40 }])
        });
        let sharp = FocusMap::compute(&texture);
        let blurred = FocusMap::compute(&image::imageops::blur(&texture, 2.0));
        assert!(sharp.score(region) > blurred.score(region) * 1.5);
        let crop = egui::Rect::from_min_max(egui::pos2(16.0, 16.0), egui::pos2(96.0, 96.0));
        assert!(sharp.score(crop) > blurred.score(crop));
        assert_eq!(sharp.energy, FocusMap::compute(&texture).energy);
    }
    #[test]
    fn native_gradients_distinguish_blur_and_ignore_flat_fields() {
        let sharp = GrayImage::from_fn(128, 128, |x, y| {
            image::Luma([if (x / 8 + y / 8) % 2 == 0 { 220 } else { 30 }])
        });
        let blur = image::imageops::blur(&sharp, 3.0);
        let region = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(128.0, 128.0));
        assert!(FocusMap::compute(&sharp).score(region) > FocusMap::compute(&blur).score(region));
        assert_eq!(
            FocusMap::compute(&GrayImage::from_pixel(128, 128, image::Luma([100]))).score(region),
            0.0
        );
    }
}
