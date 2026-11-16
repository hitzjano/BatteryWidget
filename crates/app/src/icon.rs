//! The tray icon is drawn at run time: a battery whose fill shows the lowest level of all
//! connected devices. This module produces the pixels; `tray` turns them into an `HICON`.

use crate::theme;

/// Premultiplied BGRA, top-down, `size * size * 4` bytes.
pub fn battery_icon(
    size: u32,
    level: Option<u8>,
    charging: bool,
    light_taskbar: bool,
    low_threshold: u8,
) -> Vec<u8> {
    let size = size.max(8) as i32;
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    let unit = size as f32 / 16.0;
    let px = |v: f32| (v * unit).round() as i32;
    let outline = if light_taskbar { 0x1B1B1B } else { 0xFFFFFF };
    let mut fill = |x0: i32, y0: i32, x1: i32, y1: i32, color: u32| {
        for y in y0.max(0)..y1.min(size) {
            for x in x0.max(0)..x1.min(size) {
                let at = ((y * size + x) * 4) as usize;
                pixels[at] = (color & 0xFF) as u8;
                pixels[at + 1] = ((color >> 8) & 0xFF) as u8;
                pixels[at + 2] = ((color >> 16) & 0xFF) as u8;
                pixels[at + 3] = 0xFF;
            }
        }
    };

    // Body 13 x 8 units, centred vertically, with a terminal on the right.
    let (left, top, right, bottom) = (px(1.0), px(4.0), px(13.0), px(12.0));
    let border = px(1.0).max(1);
    fill(left, top, right, top + border, outline);
    fill(left, bottom - border, right, bottom, outline);
    fill(left, top, left + border, bottom, outline);
    fill(right - border, top, right, bottom, outline);
    fill(right, px(6.0), right + px(2.0).max(1), px(10.0), outline);

    if let Some(level) = level {
        let gap = border;
        let (inner_left, inner_right) = (left + border + gap, right - border - gap);
        let span = (inner_right - inner_left).max(1);
        // Always show at least a sliver so that an almost empty battery is not blank.
        let filled =
            ((span as f32 * f32::from(level.min(100)) / 100.0).round() as i32).clamp(1, span);
        let color = theme::level_color(level, charging, low_threshold);
        fill(
            inner_left,
            top + border + gap,
            inner_left + filled,
            bottom - border - gap,
            color,
        );
    }
    pixels
}

/// The level the tray icon represents: the lowest among devices that are in use.
pub fn summary_level(devices: &[battery_widget_core::Device]) -> Option<(u8, bool)> {
    devices
        .iter()
        .filter(|d| d.connected && !d.idle)
        .filter_map(|d| d.battery.map(|level| (level, d.charging)))
        .min_by_key(|(level, _)| *level)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(pixels: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * size + x) * 4) as usize;
        [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
    }

    #[test]
    fn outline_and_transparency() {
        let icon = battery_icon(16, None, false, false, 20);
        assert_eq!(icon.len(), 16 * 16 * 4);
        assert_eq!(
            pixel(&icon, 16, 0, 0),
            [0, 0, 0, 0],
            "corners stay transparent"
        );
        assert_eq!(
            pixel(&icon, 16, 1, 4),
            [0xFF, 0xFF, 0xFF, 0xFF],
            "white outline on a dark taskbar"
        );
        assert_eq!(
            pixel(&icon, 16, 6, 8),
            [0, 0, 0, 0],
            "no fill without a level"
        );
        assert_eq!(
            pixel(&battery_icon(16, None, false, true, 20), 16, 1, 4),
            [0x1B, 0x1B, 0x1B, 0xFF]
        );
    }

    #[test]
    fn fill_follows_level() {
        let full = battery_icon(32, Some(100), false, false, 20);
        let low = battery_icon(32, Some(10), false, false, 20);
        let count = |pixels: &[u8], bgr: [u8; 3]| {
            pixels
                .chunks(4)
                .filter(|p| p[..3] == bgr && p[3] == 0xFF)
                .count()
        };
        let green = [0x72, 0xB8, 0x2E];
        let red = [0x4D, 0x48, 0xE5];
        assert!(count(&full, green) > 100);
        assert_eq!(count(&full, red), 0);
        assert!(count(&low, red) > 0 && count(&low, red) < count(&full, green) / 4);
        assert!(
            count(&battery_icon(32, Some(10), true, false, 20), green) > 0,
            "charging is green"
        );
    }

    #[test]
    fn odd_sizes_do_not_panic() {
        for size in [0, 1, 8, 20, 24, 40, 64] {
            let side = size.max(8);
            assert_eq!(
                battery_icon(size, Some(55), false, true, 20).len(),
                (side * side * 4) as usize
            );
        }
    }
}
