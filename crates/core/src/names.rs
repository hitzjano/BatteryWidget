//! Name handling: guessing a device kind from its name and deciding whether two names
//! reported by different sources describe the same physical device.

use crate::model::Kind;

const REMOTE: &[&str] = &["remote", "fire tv", "siri remote"];
const GAMEPAD: &[&str] = &[
    "controller",
    "gamepad",
    "joystick",
    "xbox",
    "dualsense",
    "dualshock",
    "joy-con",
    "switch pro",
    "stadia",
    "8bitdo",
    "flydigi",
    "gamesir",
    "gulikit",
    "scuf",
    "wolverine",
    "racing wheel",
    "driving force",
    "f710",
    "f310",
    "g923",
    "g29",
    "g920",
];
const SPEAKER: &[&str] = &[
    "speaker",
    "soundbar",
    "jbl flip",
    "jbl charge",
    "jbl xtreme",
    "jbl go",
    "jbl clip",
    "boombox",
    "partybox",
    "megaboom",
    "wonderboom",
    "ue boom",
    "soundlink",
    "soundcore motion",
    "emberton",
    "stanmore",
    "homepod",
    "echo dot",
    "g560",
    "z407",
];
const MOUSE: &[&str] = &[
    "mouse",
    "trackball",
    "superlight",
    "mx master",
    "mx anywhere",
    "mx ergo",
    "mx vertical",
    "lift",
    "m575",
    "m650",
    "m720",
    "m590",
    "m330",
    "pebble m",
    "g502",
    "g305",
    "g304",
    "g309",
    "g703",
    "g603",
    "g903",
    "g604",
    "g705",
    "g403",
    "g900",
    "pro wireless",
    "viper",
    "deathadder",
    "basilisk",
    "naga",
    "orochi",
    "aerox",
    "rival",
    "pulsefire",
    "dark core",
    "ironclaw",
    "harpoon",
    "katar",
    "scimitar",
    "model o",
    "model d",
];
const WATCH: &[&str] = &[
    "watch",
    "smartband",
    "mi band",
    "galaxy fit",
    "fitbit",
    "amazfit",
    "forerunner",
    "fenix",
];
const MICROPHONE: &[&str] = &["microphone", "dji mic", "rode wireless", "wireless go"];
const KEYBOARD: &[&str] = &[
    "keyboard",
    "mx keys",
    "mx mechanical",
    "pop keys",
    "craft",
    "k380",
    "k780",
    "k580",
    "k400",
    "k270",
    "k360",
    "k800",
    "k850",
    "k860",
    "k950",
    "g915",
    "g815",
    "g613",
    "g715",
    "g515",
    "pro x tkl",
    "pro x 60",
    "keychron",
    "nuphy",
    "blackwidow",
    "huntsman",
    "deathstalker",
    "k57",
    "k63",
    "k70",
    "k100",
    "apex pro",
    "alloy",
    "falchion",
    "azoth",
    "aula",
    "epomaker",
    "royal kludge",
    "ducky",
    "wooting",
    " kb",
];
const HEADSET: &[&str] = &[
    "headset",
    "headphone",
    "earphone",
    "earbud",
    "buds",
    "airpods",
    "pods",
    "wh-",
    "wf-",
    "hands-free",
    "jabra",
    "quietcomfort",
    "qc35",
    "qc45",
    "qc ultra",
    "bose",
    "beats",
    "linkbuds",
    "inzone",
    "pulse 3d",
    "arctis",
    "cloud ii",
    "cloud alpha",
    "cloud flight",
    "cloud iii",
    "void",
    "virtuoso",
    "hs70",
    "hs80",
    "kraken",
    "blackshark",
    "barracuda",
    "g733",
    "g533",
    "g933",
    "g935",
    "g535",
    "g435",
    "g735",
    "g522",
    "pro x",
    "astro",
    "a50",
    "a30",
    "a20",
    "zone vibe",
    "zone wireless",
    "momentum",
    "freebuds",
    "soundcore life",
    "soundcore liberty",
    "soundcore space",
    "galaxy buds",
    "pixel buds",
    "tune ",
    "live pro",
    "elite 7",
    "elite 8",
    "elite 10",
    "evolve",
    "maxwell",
    "penrose",
    "stealth 600",
    "stealth 700",
    "h800",
    "h600",
];
const MISC: &[(Kind, &[&str])] = &[
    (
        Kind::Phone,
        &[
            "iphone",
            "smartphone",
            "galaxy s",
            "pixel 6",
            "pixel 7",
            "pixel 8",
            "pixel 9",
        ],
    ),
    (Kind::Tablet, &["ipad", "tablet", "galaxy tab", "kindle"]),
    (
        Kind::Pen,
        &["surface pen", "slim pen", "s pen", "stylus", "pencil"],
    ),
    (
        Kind::Computer,
        &[
            "laptop", "notebook", "macbook", "imac", "thinkpad", "desktop-",
        ],
    ),
    (
        Kind::Camera,
        &["camera", "gopro", "insta360", "dji osmo", "dji pocket"],
    ),
    (Kind::Tv, &["television", "bravia", "chromecast", " tv"]),
];

fn any(n: &str, words: &[&str]) -> bool {
    words.iter().any(|w| n.contains(w))
}

/// Best-effort device category from a product name.
pub fn kind_from_name(name: &str) -> Kind {
    let n = format!(" {} ", name.trim().to_lowercase());
    let n = n.as_str();
    if any(n, REMOTE) {
        Kind::Remote
    } else if any(n, GAMEPAD) {
        Kind::Gamepad
    } else if any(n, SPEAKER) {
        Kind::Speaker
    } else if any(n, MOUSE) {
        Kind::Mouse
    } else if any(n, WATCH) {
        Kind::Watch
    } else if any(n, MICROPHONE) {
        Kind::Microphone
    } else if any(n, KEYBOARD) {
        Kind::Keyboard
    } else if any(n, HEADSET) {
        Kind::Headset
    } else {
        MISC.iter()
            .find(|(_, words)| any(n, words))
            .map(|(k, _)| *k)
            .unwrap_or(Kind::Generic)
    }
}

const NAME_NOISE: &[&str] = &[
    "lightspeed",
    "wireless",
    "gaming",
    "mechanical",
    "rgb",
    "bluetooth",
    "keyboard",
    "mouse",
];
const RADIO_SUFFIX: &[&str] = &["3.0", "5.0", "ble"];
const TYPE_SUFFIX: &[&str] = &["kb", "keyboard", "mouse"];

/// `"G915 TKL LIGHTSPEED Wireless RGB Mechanical Gaming Keyboard"` → `"G915 TKL"`,
/// `"AULA-F75 5.0 KB"` → `"AULA-F75"`.
pub fn short_name(name: &str) -> String {
    let name = name.trim();
    let mut tokens: Vec<&str> = name.split_whitespace().collect();
    // Everything from the first marketing word onwards is noise (never the first token).
    if let Some(cut) = tokens.iter().skip(1).position(|t| {
        let t = t
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase();
        NAME_NOISE.contains(&t.as_str())
    }) {
        tokens.truncate(cut + 1);
    }
    let ends_with = |tokens: &[&str], set: &[&str]| {
        tokens.len() > 1
            && tokens
                .last()
                .is_some_and(|t| set.contains(&t.to_lowercase().as_str()))
    };
    if ends_with(&tokens, TYPE_SUFFIX) {
        tokens.pop();
    }
    if ends_with(&tokens, RADIO_SUFFIX) {
        tokens.pop();
    }
    let short = tokens.join(" ");
    if short.chars().count() >= 3 {
        short
    } else {
        name.to_string()
    }
}

const BRANDS: &[&str] = &[
    "logitech ",
    "corsair ",
    "razer ",
    "steelseries ",
    "hyperx ",
    "epomaker ",
    "aula ",
    "8bitdo ",
    "sony ",
    "asus ",
    "glorious ",
    "roccat ",
    "keychron ",
    "nuphy ",
    "royal kludge ",
    "redragon ",
    "ducky ",
    "wooting ",
    "microsoft ",
];

/// Normalised name used for de-duplication: lower-case, no brand prefix, no marketing suffix.
pub fn norm_device_name(name: &str) -> String {
    let lowered = short_name(name).to_lowercase();
    let mut s = lowered
        .split(|c: char| c.is_whitespace() || c == '-' || c == '_')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    for brand in BRANDS {
        if s.len() > brand.len() && s.starts_with(brand) {
            s = s[brand.len()..].to_string();
        }
    }
    s.trim().to_string()
}

const GENERIC_WORDS: &[&str] = &[
    "keyboard",
    "audio",
    "series",
    "device",
    "mouse",
    "wireless",
    "edition",
    "gaming",
    "headset",
    "controller",
    "bluetooth",
];

fn alnum_tokens(s: &str) -> Vec<&str> {
    s.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Do two names (optionally with their kinds) describe the same physical device?
///
/// Identical normalised names match. Otherwise one name must contain the other and the
/// remainder may only consist of generic words: `"MX Keys"` matches `"MX Keys Keyboard"`
/// but not `"MX Keys Mini"` or `"MX Keys 2"`.
pub fn is_same_physical_device(n1: &str, k1: Kind, n2: &str, k2: Kind) -> bool {
    if k1 != k2 && k1 != Kind::Generic && k2 != Kind::Generic {
        return false;
    }
    let (s1, s2) = (norm_device_name(n1), norm_device_name(n2));
    if s1.is_empty() || s2.is_empty() {
        return false;
    }
    if s1 == s2 {
        return true;
    }
    let (short, long) = if s1.len() <= s2.len() {
        (&s1, &s2)
    } else {
        (&s2, &s1)
    };
    if short.len() < 3 {
        return false;
    }
    let padded = format!(" {long} ");
    let Some(pos) = padded.find(&format!(" {short} ")) else {
        return false;
    };
    let rest = format!("{} {}", &padded[..pos], &padded[pos + short.len() + 1..]);
    alnum_tokens(&rest)
        .iter()
        .all(|t| GENERIC_WORDS.contains(t) && !t.chars().all(|c| c.is_ascii_digit()))
}

const PNP_PREFIXES: &[&str] = &["avrcp transport for ", "transporte avrcp para "];
const PNP_SUFFIXES: &[&str] = &[
    " hands-free ag audio",
    " hands-free ag",
    " hands-free",
    " stereo",
    " avrcp transport",
    " avrcpcustom",
    " ag audio",
    " audio",
];

/// `"WH-1000XM4 Hands-Free AG"` → `"WH-1000XM4"`. Windows lists one PnP node per profile.
pub fn clean_pnp_name(name: &str) -> String {
    let original = name.trim();
    let mut n = original;
    for prefix in PNP_PREFIXES {
        let matches = n
            .get(..prefix.len())
            .is_some_and(|p| p.eq_ignore_ascii_case(prefix));
        if matches && n.len() > prefix.len() {
            n = n[prefix.len()..].trim();
        }
    }
    let lower = n.to_ascii_lowercase();
    if let Some(cut) = PNP_SUFFIXES.iter().filter_map(|s| lower.find(s)).min() {
        n = n[..cut].trim();
    }
    if n.chars().count() >= 2 {
        n.to_string()
    } else {
        original.to_string()
    }
}

/// `"CORSAIR DARK CORE RGB Wireless Gaming Dongle"` → `"Corsair Dark Core RGB"`.
pub fn clean_product_name(product: &str, fallback: &str) -> String {
    const NOISE: &[&str] = &[
        "wireless",
        "gaming",
        "dongle",
        "receiver",
        "usb",
        "rf",
        "mouse",
        "keyboard",
        "headset",
        "controller",
        "mechanical",
    ];
    let words: Vec<String> = product
        .split_whitespace()
        .filter(|w| !NOISE.contains(&w.to_lowercase().as_str()))
        .map(|w| {
            let shouting = w.len() > 3 && w.chars().all(|c| c.is_ascii_uppercase());
            if shouting {
                format!("{}{}", &w[..1], w[1..].to_lowercase())
            } else {
                w.to_string()
            }
        })
        .collect();
    let name = words.join(" ");
    let name = name.trim_matches(|c| c == ' ' || c == '-' || c == '_');
    if name.chars().count() >= 3 {
        name.to_string()
    } else {
        fallback.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert_eq!(kind_from_name("MX Master 3S"), Kind::Mouse);
        assert_eq!(kind_from_name("MX Keys Mini"), Kind::Keyboard);
        assert_eq!(kind_from_name("WH-1000XM4"), Kind::Headset);
        assert_eq!(kind_from_name("Xbox Wireless Controller"), Kind::Gamepad);
        assert_eq!(kind_from_name("JBL Flip 6"), Kind::Speaker);
        assert_eq!(kind_from_name("Galaxy Watch6"), Kind::Watch);
        assert_eq!(kind_from_name("Something"), Kind::Generic);
    }

    #[test]
    fn short_names() {
        assert_eq!(
            short_name("G915 TKL LIGHTSPEED Wireless RGB Mechanical Gaming Keyboard"),
            "G915 TKL"
        );
        assert_eq!(short_name("AULA-F75 5.0 KB"), "AULA-F75");
        assert_eq!(short_name("Wireless Controller"), "Wireless Controller");
        assert_eq!(short_name("MX Master 3S"), "MX Master 3S");
    }

    #[test]
    fn normalised() {
        assert_eq!(norm_device_name("Logitech G915 TKL LIGHTSPEED"), "g915 tkl");
        assert_eq!(norm_device_name("AULA-F75 5.0 KB"), "f75");
        assert_eq!(norm_device_name("Logitech"), "logitech");
    }

    #[test]
    fn same_device() {
        let g = Kind::Generic;
        assert!(is_same_physical_device(
            "MX Keys",
            Kind::Keyboard,
            "Logitech MX Keys Keyboard",
            g
        ));
        assert!(is_same_physical_device(
            "PRO X Wireless",
            g,
            "PRO X Wireless Gaming Headset",
            g
        ));
        assert!(!is_same_physical_device("MX Keys", g, "MX Keys Mini", g));
        assert!(!is_same_physical_device("MX Master 3", g, "MX Master 2", g));
        assert!(!is_same_physical_device("MX Master", g, "MX Master 3", g));
        assert!(!is_same_physical_device(
            "G502",
            Kind::Mouse,
            "G502",
            Kind::Keyboard
        ));
        assert!(!is_same_physical_device("", g, "", g));
    }

    #[test]
    fn pnp_names() {
        assert_eq!(clean_pnp_name("ULT WEAR Hands-Free AG"), "ULT WEAR");
        assert_eq!(
            clean_pnp_name("AVRCP Transport for WH-1000XM4"),
            "WH-1000XM4"
        );
        assert_eq!(clean_pnp_name("Surface Pen"), "Surface Pen");
    }

    #[test]
    fn product_names() {
        assert_eq!(
            clean_product_name("CORSAIR DARK CORE RGB Wireless Gaming Dongle", "Corsair"),
            "Corsair Dark Core RGB"
        );
        assert_eq!(clean_product_name("USB Receiver", "Razer"), "Razer");
    }
}
