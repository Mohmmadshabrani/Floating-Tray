//! Filter tray pixels at the final DPI, with premultiplied alpha to avoid edge halos.

pub struct IconImage {
    pub width: usize,
    pub height: usize,
    pub bgra: Vec<u8>,
}

impl IconImage {
    pub fn from_rgba(rgba: &[u8], width: usize, height: usize) -> Self {
        let bgra = rgba
            .chunks_exact(4)
            .flat_map(|pixel| {
                let alpha = u16::from(pixel[3]);
                let premultiply = |color: u8| ((u16::from(color) * alpha + 127) / 255) as u8;
                [
                    premultiply(pixel[2]),
                    premultiply(pixel[1]),
                    premultiply(pixel[0]),
                    pixel[3],
                ]
            })
            .collect();
        let image = Self {
            width,
            height,
            bgra,
        };
        // A bounded cache keeps unusually large tray images from dominating RAM.
        if width.max(height) > 128 {
            image.fit(128)
        } else {
            image
        }
    }

    pub fn fit(&self, size: usize) -> Self {
        let max_side = self.width.max(self.height);
        let width = (self.width * size).div_ceil(max_side);
        let height = (self.height * size).div_ceil(max_side);
        if width == self.width && height == self.height {
            return Self {
                width,
                height,
                bgra: self.bgra.clone(),
            };
        }

        let horizontal = weights(self.width, width);
        let vertical = weights(self.height, height);
        let mut intermediate = vec![[0.0_f32; 4]; width * self.height];
        for (source, dest) in self
            .bgra
            .chunks_exact(self.width * 4)
            .zip(intermediate.chunks_exact_mut(width))
        {
            for (pixel, samples) in dest.iter_mut().zip(&horizontal) {
                for &(x, weight) in samples {
                    for (channel, value) in pixel.iter_mut().enumerate() {
                        *value += f32::from(source[x * 4 + channel]) * weight;
                    }
                }
            }
        }

        let mut bgra = Vec::with_capacity(width * height * 4);
        for samples in &vertical {
            for x in 0..width {
                let mut pixel = [0.0_f32; 4];
                for &(y, weight) in samples {
                    for (value, source) in pixel.iter_mut().zip(intermediate[y * width + x]) {
                        *value += source * weight;
                    }
                }
                // Lanczos can overshoot at hard edges. Keep the result premultiplied.
                let alpha = pixel[3].round().clamp(0.0, 255.0);
                bgra.extend(
                    pixel[..3]
                        .iter()
                        .map(|value| value.round().clamp(0.0, alpha) as u8),
                );
                bgra.push(alpha as u8);
            }
        }
        Self {
            width,
            height,
            bgra,
        }
    }
}

fn weights(source: usize, dest: usize) -> Vec<Vec<(usize, f32)>> {
    let scale = source as f32 / dest as f32;
    let support = scale.max(1.0);
    (0..dest)
        .map(|output| {
            let center = (output as f32 + 0.5) * scale - 0.5;
            let first = ((center - 3.0 * support).ceil() as usize).min(source - 1);
            let last = ((center + 3.0 * support).floor() as usize).min(source - 1);
            let sinc = |x: f32| {
                let angle = std::f32::consts::PI * x;
                if angle.abs() < f32::EPSILON {
                    1.0
                } else {
                    angle.sin() / angle
                }
            };
            let mut samples: Vec<_> = (first..=last)
                .map(|input| {
                    let x = (input as f32 - center) / support;
                    (input, sinc(x) * sinc(x / 3.0))
                })
                .collect();
            let total: f32 = samples.iter().map(|(_, weight)| weight).sum();
            for (_, weight) in &mut samples {
                *weight /= total;
            }
            samples
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_without_distortion_and_filters_transparent_pixels() {
        let icon = IconImage::from_rgba(&[255, 0, 0, 255, 0, 255, 0, 0], 2, 1);
        assert_eq!(icon.bgra, [0, 0, 255, 255, 0, 0, 0, 0]);
        let enlarged = icon.fit(8);
        assert_eq!((enlarged.width, enlarged.height), (8, 4));
        assert!(
            enlarged
                .bgra
                .chunks_exact(4)
                .any(|pixel| (1..255).contains(&pixel[3]))
        );
        assert!(
            enlarged
                .bgra
                .chunks_exact(4)
                .all(|pixel| pixel[1] == 0 && pixel[2] == pixel[3])
        );
        let reduced = icon.fit(1);
        assert_eq!(reduced.bgra, [0, 0, 128, 128]);
    }
}
