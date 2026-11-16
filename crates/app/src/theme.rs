//! Colours. Kept free of platform types so the choices are testable.

/// Straight (non-premultiplied) RGBA, components in `0.0..=1.0`.
pub type Rgba = [f32; 4];

const fn rgb(hex: u32, alpha: f32) -> Rgba {
    [
        ((hex >> 16) & 0xFF) as f32 / 255.0,
        ((hex >> 8) & 0xFF) as f32 / 255.0,
        (hex & 0xFF) as f32 / 255.0,
        alpha,
    ]
}

pub const GOOD: u32 = 0x2EB872;
pub const MEDIUM: u32 = 0xE6A100;
pub const LOW: u32 = 0xE5484D;
/// Below this level (and above the low threshold) the battery is shown in amber.
pub const MEDIUM_BELOW: u8 = 50;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub text: Rgba,
    pub pill: Rgba,
    pub border: Rgba,
    /// Battery outline and device icon.
    pub glyph: Rgba,
}

/// `light`: light background behind the widget. `solid`: the widget floats over arbitrary
/// content (not the taskbar) and needs an opaque background of its own to stay legible.
pub fn palette(light: bool, solid: bool) -> Palette {
    match (light, solid) {
        (false, false) => Palette {
            text: rgb(0xFFFFFF, 1.0),
            pill: rgb(0xFFFFFF, 0.07),
            border: rgb(0xFFFFFF, 0.0),
            glyph: rgb(0xFFFFFF, 0.86),
        },
        (true, false) => Palette {
            text: rgb(0x1B1B1B, 1.0),
            pill: rgb(0x000000, 0.05),
            border: rgb(0x000000, 0.0),
            glyph: rgb(0x1B1B1B, 0.82),
        },
        (false, true) => Palette {
            text: rgb(0xFFFFFF, 1.0),
            pill: rgb(0x202020, 0.94),
            border: rgb(0xFFFFFF, 0.10),
            glyph: rgb(0xFFFFFF, 0.86),
        },
        (true, true) => Palette {
            text: rgb(0x1B1B1B, 1.0),
            pill: rgb(0xF6F6F6, 0.96),
            border: rgb(0x000000, 0.10),
            glyph: rgb(0x1B1B1B, 0.82),
        },
    }
}

/// Fill colour of the battery glyph as `0xRRGGBB`.
pub fn level_color(level: u8, charging: bool, low_threshold: u8) -> u32 {
    if charging {
        GOOD
    } else if level <= low_threshold {
        LOW
    } else if level < MEDIUM_BELOW {
        MEDIUM
    } else {
        GOOD
    }
}

pub fn opaque(hex: u32) -> Rgba {
    rgb(hex, 1.0)
}

pub fn with_alpha(color: Rgba, factor: f32) -> Rgba {
    [color[0], color[1], color[2], color[3] * factor]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_colours() {
        assert_eq!(level_color(80, false, 20), GOOD);
        assert_eq!(level_color(50, false, 20), GOOD);
        assert_eq!(level_color(49, false, 20), MEDIUM);
        assert_eq!(level_color(20, false, 20), LOW);
        assert_eq!(
            level_color(5, true, 20),
            GOOD,
            "charging is always shown as good"
        );
    }

    #[test]
    fn hex_conversion() {
        assert_eq!(opaque(0xFF0080), [1.0, 0.0, 128.0 / 255.0, 1.0]);
        assert_eq!(with_alpha(opaque(0xFFFFFF), 0.5)[3], 0.5);
    }
}
