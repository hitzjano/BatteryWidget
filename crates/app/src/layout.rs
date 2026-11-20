//! Geometry of the widget: one "pill" per device, laid out in a row.
//!
//! Pure arithmetic; text widths come from a caller-supplied measuring function, so the
//! layout rules are testable without DirectWrite.

use battery_widget_core::Device;
use serde::{Deserialize, Serialize};

/// All sizes in device-independent pixels (multiplied by the DPI scale at run time).
pub const HEIGHT: f32 = 28.0;
pub const PILL_PADDING: f32 = 7.0;
pub const PILL_GAP: f32 = 4.0;
pub const ICON_BOX: f32 = 16.0;
pub const ITEM_GAP: f32 = 5.0;
pub const BATTERY_WIDTH: f32 = 22.0;
pub const BATTERY_HEIGHT: f32 = 11.0;
pub const BATTERY_NUB: f32 = 2.0;
pub const TEXT_SIZE: f32 = 12.0;
pub const ICON_SIZE: f32 = 13.0;
pub const RADIUS: f32 = 6.0;
/// Battery large enough to carry the percentage inside ([`Display::BarPercent`]).
pub const BIG_BATTERY_WIDTH: f32 = 36.0;
pub const BIG_BATTERY_HEIGHT: f32 = 16.0;
pub const INNER_TEXT_SIZE: f32 = 9.5;
/// Room for the charging bolt where it cannot sit on the battery bar.
pub const BOLT_SLOT: f32 = 10.0;
/// Space kept free above and below the widget in a taskbar lower than that.
const TASKBAR_INSET: f32 = 5.0;
/// Below this the widget would become illegible.
const MIN_FIT: f32 = 0.7;

/// Size factor for the widget inside a taskbar `bar_height` device-independent pixels
/// high: `1.0` for a standard taskbar (48 on Windows 11, 40 on Windows 10), smaller for compact ones ("small taskbar buttons")
/// so that the widget keeps some air above and below.
pub fn taskbar_fit(bar_height: f32) -> f32 {
    ((bar_height - 2.0 * TASKBAR_INSET) / HEIGHT).clamp(MIN_FIT, 1.0)
}

/// Name lengths tried in turn when space is tight.
const NAME_LIMITS: [usize; 3] = [18, 12, 8];

/// What each pill shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Display {
    /// Device icon, battery bar and percentage.
    #[default]
    Full,
    /// Device icon and battery bar.
    IconBar,
    /// Device icon and percentage.
    IconPercent,
    /// Device icon and a battery with the percentage written inside.
    BarPercent,
}

impl Display {
    pub const ALL: [(Display, &'static str); 4] = [
        (Display::Full, "Icon, battery and percentage"),
        (Display::IconBar, "Icon and battery"),
        (Display::IconPercent, "Icon and percentage"),
        (
            Display::BarPercent,
            "Icon and battery with percentage inside",
        ),
    ];

    /// The charging bolt is drawn on the battery bar where there is one that is not
    /// covered by text; otherwise it needs a slot of its own.
    pub fn bolt_beside(self) -> bool {
        matches!(self, Display::IconPercent | Display::BarPercent)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pill {
    pub x: f32,
    pub width: f32,
    /// Index into the device slice; `None` for the "+N" overflow pill.
    pub device: Option<usize>,
    pub name: Option<String>,
    pub name_width: f32,
    pub label: String,
    pub label_width: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub pills: Vec<Pill>,
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    pub display: Display,
}

/// `"85%"`, or an en dash while the level is unknown.
pub fn percent_label(device: &Device) -> String {
    device
        .battery
        .map_or_else(|| "–".to_string(), |level| format!("{level}%"))
}

pub fn ellipsize(name: &str, max_chars: usize) -> String {
    if name.chars().count() <= max_chars {
        return name.to_string();
    }
    let head: String = name.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{}…", head.trim_end())
}

/// The free stretches of `start..end` that none of the `occupied` spans cover, left to right.
pub fn free_gaps(start: i32, end: i32, occupied: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut spans: Vec<(i32, i32)> = occupied.to_vec();
    spans.sort_unstable();
    let mut gaps = Vec::new();
    let mut cursor = start;
    for (left, right) in spans {
        if left > cursor && cursor < end {
            gaps.push((cursor, left.min(end)));
        }
        cursor = cursor.max(right);
    }
    if cursor < end {
        gaps.push((cursor, end));
    }
    gaps
}

fn build(
    devices: &[Device],
    selected: &[usize],
    // Show names, shortened to this many characters.
    names: Option<usize>,
    display: Display,
    scale: f32,
    measure: &dyn Fn(&str) -> f32,
) -> Layout {
    let s = |v: f32| v * scale;
    let mut pills: Vec<Pill> = Vec::new();
    let mut cursor = 0.0;
    for (index, device) in selected.iter().filter_map(|i| Some((*i, devices.get(*i)?))) {
        let label = percent_label(device);
        let label_width = measure(&label).ceil();
        let name = names.map(|limit| ellipsize(&device.name, limit));
        let name_width = name.as_deref().map_or(0.0, |n| measure(n).ceil());
        let mut width = s(PILL_PADDING) * 2.0 + s(ICON_BOX) + s(ITEM_GAP);
        if name.is_some() {
            width += name_width + s(ITEM_GAP);
        }
        width += match display {
            Display::Full => s(BATTERY_WIDTH + BATTERY_NUB) + s(ITEM_GAP) + label_width,
            Display::IconBar => s(BATTERY_WIDTH + BATTERY_NUB),
            Display::IconPercent => label_width,
            Display::BarPercent => s(BIG_BATTERY_WIDTH + BATTERY_NUB),
        };
        if device.charging && display.bolt_beside() {
            width += s(BOLT_SLOT);
        }
        let width = width.ceil();
        pills.push(Pill {
            x: cursor,
            width,
            device: Some(index),
            name,
            name_width,
            label,
            label_width,
        });
        cursor += width + s(PILL_GAP);
    }
    if selected.len() < devices.len() {
        let label = format!("+{}", devices.len() - selected.len());
        let label_width = measure(&label).ceil();
        let width = (s(PILL_PADDING) * 2.0 + label_width).ceil();
        pills.push(Pill {
            x: cursor,
            width,
            device: None,
            name: None,
            name_width: 0.0,
            label,
            label_width,
        });
        cursor += width + s(PILL_GAP);
    }
    let width = if pills.is_empty() {
        0.0
    } else {
        (cursor - s(PILL_GAP)).ceil()
    };
    Layout {
        pills,
        width,
        height: s(HEIGHT).ceil(),
        scale,
        display,
    }
}

/// The `count` devices that most need attention, in their original order: devices in use
/// come before sleeping ones, lower levels before higher ones, unknown levels last.
pub fn most_urgent(devices: &[Device], count: usize) -> Vec<usize> {
    let mut ranked: Vec<usize> = (0..devices.len()).collect();
    ranked.sort_by_key(|i| {
        let device = &devices[*i];
        (
            device.idle || !device.connected,
            device.battery.unwrap_or(u8::MAX),
            *i,
        )
    });
    ranked.truncate(count);
    ranked.sort_unstable();
    ranked
}

/// Lay out `devices`, degrading gracefully when space is tight.
///
/// Names are a deliberate choice of the user, so they are kept as long as possible:
/// first they are shortened, then devices collapse into a "+N" pill (the weakest
/// batteries stay visible), and only if not even one named device fits are they dropped.
pub fn compute(
    devices: &[Device],
    show_names: bool,
    display: Display,
    max_width: f32,
    scale: f32,
    measure: &dyn Fn(&str) -> f32,
) -> Layout {
    let all: Vec<usize> = (0..devices.len()).collect();
    let fits = |layout: &Layout| layout.width <= max_width;
    if show_names && !devices.is_empty() {
        for limit in NAME_LIMITS {
            let layout = build(devices, &all, Some(limit), display, scale, measure);
            if fits(&layout) {
                return layout;
            }
        }
        // Fewer devices, each with the longest name that still fits.
        for count in (1..devices.len()).rev() {
            let chosen = most_urgent(devices, count);
            for limit in NAME_LIMITS {
                let layout = build(devices, &chosen, Some(limit), display, scale, measure);
                if fits(&layout) {
                    return layout;
                }
            }
        }
    }
    let mut layout = build(devices, &all, None, display, scale, measure);
    let mut count = devices.len();
    while !fits(&layout) && count > 1 {
        count -= 1;
        layout = build(
            devices,
            &most_urgent(devices, count),
            None,
            display,
            scale,
            measure,
        );
    }
    layout
}

#[cfg(test)]
mod tests {
    use super::*;
    use battery_widget_core::{Kind, Link};

    fn device(name: &str, battery: Option<u8>) -> Device {
        Device {
            id: name.into(),
            name: name.into(),
            battery,
            kind: Kind::Mouse,
            link: Link::Bolt,
            connected: true,
            charging: false,
            full: false,
            charging_estimated: false,
            idle: false,
        }
    }

    fn measure(text: &str) -> f32 {
        text.chars().count() as f32 * 7.0
    }

    #[test]
    fn labels() {
        assert_eq!(percent_label(&device("a", Some(85))), "85%");
        assert_eq!(percent_label(&device("a", None)), "–");
        assert_eq!(ellipsize("MX Master 3S", 18), "MX Master 3S");
        assert_eq!(ellipsize("MX Master 3S", 8), "MX Mast…");
        assert_eq!(
            ellipsize("A very long device name indeed", 18),
            "A very long devic…"
        );
    }

    #[test]
    fn pills_are_placed_left_to_right() {
        let devices = [device("Mouse", Some(85)), device("Keyboard", Some(100))];
        let layout = compute(&devices, false, Display::Full, 1000.0, 1.0, &measure);
        assert_eq!(layout.pills.len(), 2);
        assert_eq!(layout.pills[0].x, 0.0);
        assert_eq!(layout.pills[0].width, 14.0 + 16.0 + 5.0 + 24.0 + 5.0 + 21.0);
        assert_eq!(layout.pills[1].x, layout.pills[0].width + PILL_GAP);
        assert_eq!(layout.width, layout.pills[1].x + layout.pills[1].width);
        assert_eq!(layout.height, HEIGHT);
    }

    #[test]
    fn scale_grows_everything() {
        let devices = [device("Mouse", Some(85))];
        let normal = compute(&devices, false, Display::Full, 1000.0, 1.0, &measure);
        let large = compute(&devices, false, Display::Full, 1000.0, 2.0, &|t| {
            measure(t) * 2.0
        });
        assert_eq!(large.width, normal.width * 2.0);
        assert_eq!(large.height, normal.height * 2.0);
    }

    #[test]
    fn names_are_shortened_then_devices_collapse_then_names_go() {
        let devices = [
            device("Wireless Mouse Pro", Some(85)),
            device("Mechanical Keyboard", Some(100)),
        ];
        let roomy = compute(&devices, true, Display::Full, 1000.0, 1.0, &measure);
        assert_eq!(roomy.pills[0].name.as_deref(), Some("Wireless Mouse Pro"));

        // A little too narrow: both devices stay, with shorter names.
        let tight = compute(
            &devices,
            true,
            Display::Full,
            roomy.width - 1.0,
            1.0,
            &measure,
        );
        assert_eq!(tight.pills.len(), 2);
        assert_eq!(tight.pills[0].name.as_deref(), Some("Wireless Mo…"));

        // Much too narrow: one named device plus "+1" (the weaker battery wins).
        let one = compute(&devices, true, Display::Full, 180.0, 1.0, &measure);
        assert_eq!(one.pills.len(), 2);
        assert_eq!(one.pills[0].name.as_deref(), Some("Wireles…"));
        assert_eq!(one.pills[1].label, "+1");

        // With room to spare after collapsing, the remaining name is not cut short.
        let spare = compute(&devices, true, Display::Full, 260.0, 1.0, &measure);
        assert_eq!(spare.pills.len(), 2);
        assert_eq!(spare.pills[0].name.as_deref(), Some("Wireless Mouse Pro"));

        // Not even that fits: names are dropped.
        let none = compute(&devices, true, Display::Full, 100.0, 1.0, &measure);
        assert!(none.pills.iter().all(|p| p.name.is_none()));
    }

    #[test]
    fn gaps_between_taskbar_buttons() {
        // Widgets button, then start + apps in the centre; tray begins at 1305.
        let occupied = [(6, 158), (669, 714), (714, 758), (758, 890), (700, 720)];
        assert_eq!(
            free_gaps(0, 1305, &occupied),
            [(0, 6), (158, 669), (890, 1305)]
        );
        assert_eq!(free_gaps(0, 1305, &[]), [(0, 1305)]);
        assert_eq!(free_gaps(0, 100, &[(0, 100)]), []);
        assert_eq!(free_gaps(0, 100, &[(40, 300)]), [(0, 40)]);
        assert_eq!(free_gaps(0, 100, &[(150, 300)]), [(0, 100)]);
    }

    #[test]
    fn overflow_pill_replaces_devices_that_do_not_fit() {
        let devices: Vec<Device> = (0..6).map(|i| device(&format!("D{i}"), Some(50))).collect();
        let full = compute(&devices, false, Display::Full, 10_000.0, 1.0, &measure);
        let tight = compute(
            &devices,
            false,
            Display::Full,
            full.width / 2.0,
            1.0,
            &measure,
        );
        assert!(tight.width <= full.width / 2.0);
        let last = tight.pills.last().unwrap();
        assert_eq!(last.device, None);
        assert_eq!(last.label, format!("+{}", 6 - (tight.pills.len() - 1)));
    }

    #[test]
    fn weakest_batteries_stay_visible() {
        let mut devices = vec![
            device("Keyboard", Some(42)),
            device("Mouse", Some(85)),
            device("Headset", Some(60)),
            device("Gamepad", Some(15)),
            device("Asleep", Some(5)),
            device("Unknown", None),
        ];
        devices[4].idle = true;
        assert_eq!(most_urgent(&devices, 2), [0, 3], "original order is kept");
        assert_eq!(most_urgent(&devices, 3), [0, 2, 3]);
        assert_eq!(
            most_urgent(&devices, 5),
            [0, 1, 2, 3, 5],
            "sleeping devices go first"
        );
        assert_eq!(most_urgent(&devices, 99).len(), 6);

        let full = compute(&devices, false, Display::Full, 10_000.0, 1.0, &measure);
        let two = full.pills[0].width + full.pills[3].width + PILL_GAP * 2.0 + 14.0 + 14.0;
        let tight = compute(&devices, false, Display::Full, two, 1.0, &measure);
        let shown: Vec<Option<usize>> = tight.pills.iter().map(|p| p.device).collect();
        assert_eq!(shown, [Some(0), Some(3), None]);
        assert_eq!(tight.pills[2].label, "+4");
    }

    #[test]
    fn display_styles_change_the_width() {
        let mut devices = [device("Mouse", Some(85))];
        let width = |devices: &[Device], display| {
            compute(devices, false, display, 1000.0, 1.0, &measure).width
        };
        let full = width(&devices, Display::Full);
        assert_eq!(
            width(&devices, Display::IconBar),
            full - ITEM_GAP - 21.0,
            "no percentage"
        );
        assert_eq!(
            width(&devices, Display::IconPercent),
            full - ITEM_GAP - 24.0,
            "no battery bar"
        );
        assert_eq!(
            width(&devices, Display::BarPercent),
            14.0 + ICON_BOX + ITEM_GAP + BIG_BATTERY_WIDTH + BATTERY_NUB,
            "larger battery, no separate percentage"
        );

        // A charging device needs room for the bolt wherever it cannot sit on the bar.
        devices[0].charging = true;
        assert_eq!(width(&devices, Display::Full), full);
        assert_eq!(
            width(&devices, Display::IconPercent),
            full - ITEM_GAP - 24.0 + BOLT_SLOT
        );
        assert_eq!(
            width(&devices, Display::BarPercent),
            14.0 + ICON_BOX + ITEM_GAP + BIG_BATTERY_WIDTH + BATTERY_NUB + BOLT_SLOT
        );
    }

    #[test]
    fn display_names_in_config_are_stable() {
        let text = serde_json::to_string(&Display::ALL.map(|(d, _)| d)).unwrap();
        assert_eq!(text, r#"["full","icon_bar","icon_percent","bar_percent"]"#);
    }

    #[test]
    fn compact_taskbars_get_a_smaller_widget() {
        assert_eq!(taskbar_fit(48.0), 1.0, "Windows 11 standard");
        assert_eq!(taskbar_fit(40.0), 1.0, "Windows 10 standard");
        // Windows 11 with small taskbar buttons: 32 px, the widget shrinks to 22 px.
        assert_eq!(HEIGHT * taskbar_fit(32.0), 22.0);
        assert!(HEIGHT * taskbar_fit(32.0) <= 32.0 - 2.0 * TASKBAR_INSET);
        assert_eq!(taskbar_fit(10.0), MIN_FIT, "never illegibly small");
    }

    #[test]
    fn no_devices_no_size() {
        let layout = compute(&[], true, Display::Full, 100.0, 1.0, &measure);
        assert!(layout.pills.is_empty());
        assert_eq!(layout.width, 0.0);
    }
}
