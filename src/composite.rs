pub type Pixel = [u8; 4];

pub fn panel_alpha(percent: i32) -> u8 {
    // Windows passes clicks through pixels with alpha 0. One step above zero
    // keeps the blank picker controls clickable even with a clear panel.
    ((percent * 255 + 50) / 100).max(1) as u8
}

pub fn premultiply(pixel: &mut Pixel, alpha: u8) {
    pixel[..3]
        .iter_mut()
        .for_each(|channel| *channel = scale(*channel, alpha));
    pixel[3] = alpha;
}

pub fn over(destination: &mut Pixel, source: Pixel) {
    let remaining = 255 - source[3];
    for (destination, source) in destination.iter_mut().zip(source) {
        *destination = source + scale(*destination, remaining);
    }
}

pub fn clip_corners(pixels: &mut [Pixel], width: usize, height: usize, radius: usize) {
    let radius = radius.min(width / 2).min(height / 2);
    for y in 0..radius {
        for x in 0..radius {
            let dx = radius as f32 - x as f32 - 0.5;
            let dy = radius as f32 - y as f32 - 0.5;
            let coverage = (radius as f32 + 0.5 - dx.hypot(dy)).clamp(0.0, 1.0);
            let multiplier = (coverage * 255.0).round() as u8;
            for (x, y) in [
                (x, y),
                (width - 1 - x, y),
                (x, height - 1 - y),
                (width - 1 - x, height - 1 - y),
            ] {
                pixels[y * width + x]
                    .iter_mut()
                    .for_each(|value| *value = scale(*value, multiplier));
            }
        }
    }
}

fn scale(value: u8, alpha: u8) -> u8 {
    ((u16::from(value) * u16::from(alpha) + 127) / 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_opacity_never_dims_opaque_icon_pixels() {
        let opaque = [70, 40, 220, 255];
        for opacity in [0, 40, 100] {
            let mut panel = [60, 40, 25, 0];
            premultiply(&mut panel, panel_alpha(opacity));
            over(&mut panel, opaque);
            assert_eq!(panel, opaque);
        }
    }

    #[test]
    fn clipped_corners_leave_the_center_untouched() {
        let mut pixels = [[10, 20, 30, 255]; 16 * 12];
        clip_corners(&mut pixels, 16, 12, 6);
        assert_eq!(pixels[0], [0; 4]);
        assert_eq!(pixels[15], [0; 4]);
        assert_eq!(pixels[6 + 5 * 16], [10, 20, 30, 255]);
    }

    #[test]
    fn clear_panels_remain_clickable_and_preserve_icon_antialiasing() {
        let mut panel = [33, 26, 23, 0];
        premultiply(&mut panel, panel_alpha(0));
        assert_eq!(panel, [0, 0, 0, 1]);
        over(&mut panel, [0, 0, 128, 128]);
        assert_eq!(panel, [0, 0, 128, 128]);
        assert_eq!(panel_alpha(40), 102);
        assert_eq!(panel_alpha(100), 255);
    }
}
