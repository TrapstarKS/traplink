#![allow(dead_code)]

const NONE: u16 = 0xFFFF;
const EXT: u16 = 0x100;

const KEYS: &[(&str, u16, u16)] = &[
    ("KeyA", 0x1E, 0x00),
    ("KeyS", 0x1F, 0x01),
    ("KeyD", 0x20, 0x02),
    ("KeyF", 0x21, 0x03),
    ("KeyH", 0x23, 0x04),
    ("KeyG", 0x22, 0x05),
    ("KeyZ", 0x2C, 0x06),
    ("KeyX", 0x2D, 0x07),
    ("KeyC", 0x2E, 0x08),
    ("KeyV", 0x2F, 0x09),
    ("IntlBackslash", 0x56, 0x0A),
    ("KeyB", 0x30, 0x0B),
    ("KeyQ", 0x10, 0x0C),
    ("KeyW", 0x11, 0x0D),
    ("KeyE", 0x12, 0x0E),
    ("KeyR", 0x13, 0x0F),
    ("KeyY", 0x15, 0x10),
    ("KeyT", 0x14, 0x11),
    ("Digit1", 0x02, 0x12),
    ("Digit2", 0x03, 0x13),
    ("Digit3", 0x04, 0x14),
    ("Digit4", 0x05, 0x15),
    ("Digit6", 0x07, 0x16),
    ("Digit5", 0x06, 0x17),
    ("Equal", 0x0D, 0x18),
    ("Digit9", 0x0A, 0x19),
    ("Digit7", 0x08, 0x1A),
    ("Minus", 0x0C, 0x1B),
    ("Digit8", 0x09, 0x1C),
    ("Digit0", 0x0B, 0x1D),
    ("BracketRight", 0x1B, 0x1E),
    ("KeyO", 0x18, 0x1F),
    ("KeyU", 0x16, 0x20),
    ("BracketLeft", 0x1A, 0x21),
    ("KeyI", 0x17, 0x22),
    ("KeyP", 0x19, 0x23),
    ("Enter", 0x1C, 0x24),
    ("KeyL", 0x26, 0x25),
    ("KeyJ", 0x24, 0x26),
    ("Quote", 0x28, 0x27),
    ("KeyK", 0x25, 0x28),
    ("Semicolon", 0x27, 0x29),
    ("Backslash", 0x2B, 0x2A),
    ("Comma", 0x33, 0x2B),
    ("Slash", 0x35, 0x2C),
    ("KeyN", 0x31, 0x2D),
    ("KeyM", 0x32, 0x2E),
    ("Period", 0x34, 0x2F),
    ("Tab", 0x0F, 0x30),
    ("Space", 0x39, 0x31),
    ("Backquote", 0x29, 0x32),
    ("Backspace", 0x0E, 0x33),
    ("Escape", 0x01, 0x35),
    ("MetaRight", EXT | 0x5C, 0x36),
    ("MetaLeft", EXT | 0x5B, 0x37),
    ("ShiftLeft", 0x2A, 0x38),
    ("CapsLock", 0x3A, 0x39),
    ("AltLeft", 0x38, 0x3A),
    ("ControlLeft", 0x1D, 0x3B),
    ("ShiftRight", 0x36, 0x3C),
    ("AltRight", EXT | 0x38, 0x3D),
    ("ControlRight", EXT | 0x1D, 0x3E),
    ("NumpadDecimal", 0x53, 0x41),
    ("NumpadMultiply", 0x37, 0x43),
    ("NumpadAdd", 0x4E, 0x45),
    ("NumpadDivide", EXT | 0x35, 0x4B),
    ("NumpadEnter", EXT | 0x1C, 0x4C),
    ("NumpadSubtract", 0x4A, 0x4E),
    ("Numpad0", 0x52, 0x52),
    ("Numpad1", 0x4F, 0x53),
    ("Numpad2", 0x50, 0x54),
    ("Numpad3", 0x51, 0x55),
    ("Numpad4", 0x4B, 0x56),
    ("Numpad5", 0x4C, 0x57),
    ("Numpad6", 0x4D, 0x58),
    ("Numpad7", 0x47, 0x59),
    ("Numpad8", 0x48, 0x5B),
    ("Numpad9", 0x49, 0x5C),
    ("IntlRo", 0x73, 0x5E),
    ("NumpadComma", 0x7E, 0x5F),
    ("F5", 0x3F, 0x60),
    ("F6", 0x40, 0x61),
    ("F7", 0x41, 0x62),
    ("F3", 0x3D, 0x63),
    ("F8", 0x42, 0x64),
    ("F9", 0x43, 0x65),
    ("F11", 0x57, 0x67),
    ("F10", 0x44, 0x6D),
    ("F12", 0x58, 0x6F),
    ("Insert", EXT | 0x52, 0x72),
    ("Home", EXT | 0x47, 0x73),
    ("PageUp", EXT | 0x49, 0x74),
    ("Delete", EXT | 0x53, 0x75),
    ("F4", 0x3E, 0x76),
    ("End", EXT | 0x4F, 0x77),
    ("F2", 0x3C, 0x78),
    ("PageDown", EXT | 0x51, 0x79),
    ("F1", 0x3B, 0x7A),
    ("ArrowLeft", EXT | 0x4B, 0x7B),
    ("ArrowRight", EXT | 0x4D, 0x7C),
    ("ArrowDown", EXT | 0x50, 0x7D),
    ("ArrowUp", EXT | 0x48, 0x7E),
    ("ContextMenu", EXT | 0x5D, NONE),
];

const MAC_GRAVE: u16 = 0x32;
const MAC_SECTION: u16 = 0x0A;
fn iso_swap(kvk: u16, iso: bool) -> u16 {
    match kvk {
        MAC_GRAVE if iso => MAC_SECTION,
        MAC_SECTION if iso => MAC_GRAVE,
        other => other,
    }
}

pub fn codes() -> impl Iterator<Item = &'static str> {
    KEYS.iter().map(|(code, _, _)| *code)
}
pub fn code_for_scan(scan: u32, extended: bool) -> Option<&'static str> {
    let id = u16::try_from(scan).ok().filter(|s| *s <= 0xFF)? | if extended { EXT } else { 0 };
    KEYS.iter()
        .find(|(_, s, _)| *s == id)
        .map(|(code, _, _)| *code)
}
pub fn scan_for_code(code: &str) -> Option<(u16, bool)> {
    KEYS.iter()
        .find(|(c, _, _)| *c == code)
        .map(|(_, s, _)| (s & 0xFF, s & EXT != 0))
}
pub fn scan_for_mac(kvk: u16, iso: bool) -> Option<(u32, bool)> {
    let kvk = iso_swap(kvk, iso);
    KEYS.iter()
        .find(|(_, _, m)| *m == kvk && kvk != NONE)
        .map(|(_, s, _)| (u32::from(s & 0xFF), s & EXT != 0))
}
pub fn mac_for_code(code: &str, iso: bool) -> Option<u16> {
    let code = swap_command_control(code);
    KEYS.iter()
        .find(|(c, _, m)| *c == code && *m != NONE)
        .map(|(_, _, m)| iso_swap(*m, iso))
}
pub fn swap_command_control(code: &str) -> &str {
    match code {
        "ControlLeft" => "MetaLeft",
        "MetaLeft" => "ControlLeft",
        "ControlRight" => "MetaRight",
        "MetaRight" => "ControlRight",
        other => other,
    }
}

const MAC_MODIFIERS: [(u16, u64, u64); 8] = [
    (0x37, 0x0000_0008, 0x0010_0000),
    (0x36, 0x0000_0010, 0x0010_0000),
    (0x38, 0x0000_0002, 0x0002_0000),
    (0x3C, 0x0000_0004, 0x0002_0000),
    (0x3A, 0x0000_0020, 0x0008_0000),
    (0x3D, 0x0000_0040, 0x0008_0000),
    (0x3B, 0x0000_0001, 0x0004_0000),
    (0x3E, 0x0000_2000, 0x0004_0000),
];
const MAC_DEVICE_BITS: u64 = 0x0000_207F;
pub const MAC_CAPS_LOCK: u16 = 0x39;

pub fn mac_modifier_down(kvk: u16, flags: u64) -> Option<bool> {
    let (_, device, generic) = MAC_MODIFIERS.iter().find(|(k, _, _)| *k == kvk)?;
    if flags & MAC_DEVICE_BITS == 0 {
        return Some(flags & generic != 0);
    }
    Some(flags & device != 0)
}

pub fn mac_held_modifiers(flags: u64) -> impl Iterator<Item = u16> {
    MAC_MODIFIERS
        .iter()
        .enumerate()
        .filter_map(move |(index, (kvk, device, generic))| {
            if flags & generic == 0 {
                return None;
            }
            let group = MAC_MODIFIERS
                .iter()
                .filter(|(_, _, g)| g == generic)
                .fold(0, |bits, (_, d, _)| bits | d);
            if flags & group != 0 {
                return (flags & device != 0).then_some(*kvk);
            }
            let first = MAC_MODIFIERS.iter().position(|(_, _, g)| g == generic);
            (first == Some(index)).then_some(*kvk)
        })
}

pub fn mac_scroll(lines: (i64, i64), points: (f64, f64), continuous: bool) -> (i32, i32) {
    let (vertical, horizontal) = if continuous {
        let scale = |p: f64| {
            if p.is_finite() {
                (p * 12.0).round().clamp(-12_000.0, 12_000.0) as i32
            } else {
                0
            }
        };
        (scale(points.0), scale(points.1))
    } else {
        let scale = |l: i64| (l.clamp(-100, 100) * 120) as i32;
        (scale(lines.0), scale(lines.1))
    };
    (vertical, -horizontal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_have_unique_identities() {
        for (i, (code, scan, mac)) in KEYS.iter().enumerate() {
            for (other, other_scan, other_mac) in &KEYS[i + 1..] {
                assert_ne!(code, other);
                assert_ne!(scan, other_scan, "{code} / {other}");
                if *mac != NONE {
                    assert_ne!(mac, other_mac, "{code} / {other}");
                }
            }
        }
    }

    #[test]
    fn scan_codes_round_trip() {
        for (code, _, _) in KEYS {
            let (scan, extended) = scan_for_code(code).unwrap();
            assert_eq!(code_for_scan(u32::from(scan), extended), Some(*code));
        }
        assert_eq!(code_for_scan(0x48, true), Some("ArrowUp"));
        assert_eq!(code_for_scan(0x48, false), Some("Numpad8"));
        assert_eq!(code_for_scan(0x1C, true), Some("NumpadEnter"));
        assert_eq!(code_for_scan(0x148, false), None);
        assert_eq!(code_for_scan(0x45, false), None);
    }

    #[test]
    fn mac_keys_map_to_pc_scan_codes() {
        let code = |kvk, iso| scan_for_mac(kvk, iso).and_then(|(s, e)| code_for_scan(s, e));
        assert_eq!(code(0x00, false), Some("KeyA"));
        assert_eq!(code(0x0C, false), Some("KeyQ"));
        assert_eq!(code(0x08, false), Some("KeyC"));
        assert_eq!(code(0x09, false), Some("KeyV"));
        assert_eq!(code(0x1D, false), Some("Digit0"));
        assert_eq!(code(0x12, false), Some("Digit1"));
        assert_eq!(code(0x29, false), Some("Semicolon"));
        assert_eq!(code(0x27, false), Some("Quote"));
        assert_eq!(code(0x21, false), Some("BracketLeft"));
        assert_eq!(code(0x2C, false), Some("Slash"));
        assert_eq!(code(0x5E, false), Some("IntlRo"));
        assert_eq!(code(0x24, false), Some("Enter"));
        assert_eq!(code(0x33, false), Some("Backspace"));
        assert_eq!(code(0x75, false), Some("Delete"));
        assert_eq!(code(0x35, false), Some("Escape"));
        assert_eq!(code(0x7B, false), Some("ArrowLeft"));
        assert_eq!(code(0x7C, false), Some("ArrowRight"));
        assert_eq!(code(0x7D, false), Some("ArrowDown"));
        assert_eq!(code(0x7E, false), Some("ArrowUp"));
        assert_eq!(code(0x7A, false), Some("F1"));
        assert_eq!(code(0x6F, false), Some("F12"));
        assert_eq!(code(0x37, false), Some("MetaLeft"));
        assert_eq!(code(0x36, false), Some("MetaRight"));
        assert_eq!(code(0x3B, false), Some("ControlLeft"));
        assert_eq!(code(0x3A, false), Some("AltLeft"));
        assert_eq!(code(0x3D, false), Some("AltRight"));
        assert_eq!(code(0x38, false), Some("ShiftLeft"));
        assert_eq!(code(0x4C, false), Some("NumpadEnter"));
        assert_eq!(code(0x3F, false), None);
        assert_eq!(code(0xFFFF, false), None);
        assert_eq!(code(0x32, false), Some("Backquote"));
        assert_eq!(code(0x0A, false), Some("IntlBackslash"));
        assert_eq!(code(0x32, true), Some("IntlBackslash"));
        assert_eq!(code(0x0A, true), Some("Backquote"));
    }

    #[test]
    fn mac_receiver_swaps_command_and_control() {
        assert_eq!(mac_for_code("ControlLeft", false), Some(0x37));
        assert_eq!(mac_for_code("ControlRight", false), Some(0x36));
        assert_eq!(mac_for_code("MetaLeft", false), Some(0x3B));
        assert_eq!(mac_for_code("MetaRight", false), Some(0x3E));
        assert_eq!(mac_for_code("KeyC", false), Some(0x08));
        assert_eq!(mac_for_code("Backquote", true), Some(0x0A));
        assert_eq!(mac_for_code("ContextMenu", false), None);
        assert_eq!(mac_for_code("Nope", false), None);
        assert_eq!(
            swap_command_control(swap_command_control("MetaLeft")),
            "MetaLeft"
        );
    }

    #[test]
    fn mac_modifier_flags() {
        assert_eq!(mac_modifier_down(0x37, 0x0010_0008), Some(true));
        assert_eq!(mac_modifier_down(0x37, 0x0010_0010), Some(false));
        assert_eq!(mac_modifier_down(0x36, 0x0010_0010), Some(true));
        assert_eq!(mac_modifier_down(0x38, 0x0000_0100), Some(false));
        assert_eq!(mac_modifier_down(0x3B, 0x0004_0001), Some(true));
        assert_eq!(mac_modifier_down(0x3E, 0x0004_2000), Some(true));
        assert_eq!(mac_modifier_down(0x3A, 0x0008_0020), Some(true));
        assert_eq!(mac_modifier_down(0x38, 0x0002_0000), Some(true));
        assert_eq!(mac_modifier_down(MAC_CAPS_LOCK, 0x0001_0000), None);
        assert_eq!(mac_modifier_down(0x00, 0), None);
    }

    #[test]
    fn mac_held_modifiers_prefer_device_bits_and_skip_caps_lock() {
        let held = |flags| mac_held_modifiers(flags).collect::<Vec<u16>>();
        assert_eq!(held(0), Vec::<u16>::new());
        assert_eq!(held(0x0001_0000), Vec::<u16>::new());
        assert_eq!(held(0x0010_0010), vec![0x36]);
        assert_eq!(held(0x0012_0018 | 0x0000_0004), vec![0x37, 0x36, 0x3C]);
        assert_eq!(held(0x0008_0000), vec![0x3A]);
        assert_eq!(held(0x0018_0008), vec![0x37, 0x3A]);
        assert_eq!(held(0x0004_2000), vec![0x3E]);
    }

    #[test]
    fn mac_scroll_converts_to_wheel_units() {
        assert_eq!(mac_scroll((1, 0), (10.0, 0.0), false), (120, 0));
        assert_eq!(mac_scroll((-2, 0), (0.0, 0.0), false), (-240, 0));
        assert_eq!(mac_scroll((0, 1), (0.0, 0.0), false), (0, -120));
        assert_eq!(mac_scroll((0, 0), (5.0, -10.0), true), (60, 120));
        assert_eq!(mac_scroll((0, 0), (f64::NAN, 0.0), true), (0, 0));
    }
}
