//! Pure pieces of the Windows backends (scan codes, hotkey strings, exe matching).
//! Not OS-gated so the logic is unit-tested on every target.

/// Set 1 scan code of a lane key, and whether it needs `KEYEVENTF_EXTENDEDKEY` (arrows).
/// Same key names as `uinput::key_code`.
pub fn scan_code(name: &str) -> Option<(u16, bool)> {
    Some(match name.to_lowercase().as_str() {
        "a" => (0x1E, false),
        "s" => (0x1F, false),
        "w" => (0x11, false),
        "d" => (0x20, false),
        "f" => (0x21, false),
        "j" => (0x24, false),
        "k" => (0x25, false),
        "l" => (0x26, false),
        "z" => (0x2C, false),
        "x" => (0x2D, false),
        "space" => (0x39, false),
        "left" => (0x4B, true),
        "down" => (0x50, true),
        "up" => (0x48, true),
        "right" => (0x4D, true),
        _ => return None,
    })
}

pub const MOD_ALT: u32 = 0x1;
pub const MOD_CONTROL: u32 = 0x2;
pub const MOD_SHIFT: u32 = 0x4;
pub const MOD_WIN: u32 = 0x8;

/// "F8", "ctrl+shift+F9", "p" -> (modifier bits for `RegisterHotKey`, virtual key code).
pub fn parse_hotkey(s: &str) -> Option<(u32, u32)> {
    let mut mods = 0;
    let mut vk = None;
    for part in s.split('+').map(|p| p.trim().to_lowercase()) {
        match part.as_str() {
            "ctrl" | "control" => mods |= MOD_CONTROL,
            "shift" => mods |= MOD_SHIFT,
            "alt" => mods |= MOD_ALT,
            "win" | "super" | "meta" => mods |= MOD_WIN,
            k if vk.is_none() => vk = Some(vk_code(k)?),
            _ => return None,
        }
    }
    Some((mods, vk?))
}

fn vk_code(k: &str) -> Option<u32> {
    if let Some(n) = k.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
        return (1..=24).contains(&n).then(|| 0x6F + n);
    }
    match k {
        "esc" | "escape" => Some(0x1B),
        "space" => Some(0x20),
        "pause" => Some(0x13),
        "home" => Some(0x24),
        "end" => Some(0x23),
        "insert" => Some(0x2D),
        "delete" => Some(0x2E),
        _ => {
            let mut c = k.chars();
            match (c.next(), c.next()) {
                // Letters and digits: the VK code is the upper-case ASCII value.
                (Some(ch), None) if ch.is_ascii_alphanumeric() => {
                    Some(ch.to_ascii_uppercase() as u32)
                }
                _ => None,
            }
        }
    }
}

pub use arrowhead_core::input::exe_matches;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lane_keys_have_scan_codes() {
        use arrowhead_core::config::KeySet;
        let none: [String; 4] = Default::default();
        for set in [KeySet::Wasd, KeySet::Arrows] {
            for n in arrowhead_core::input::key_names(set, &none) {
                assert!(scan_code(n).is_some(), "{n}");
            }
        }
        assert_eq!(scan_code("W"), Some((0x11, false)));
        assert_eq!(scan_code("left"), Some((0x4B, true)));
        assert_eq!(scan_code("nope"), None);
    }

    #[test]
    fn only_arrows_are_extended() {
        for n in ["a", "s", "w", "d", "f", "j", "k", "l", "z", "x", "space"] {
            assert!(!scan_code(n).unwrap().1, "{n}");
        }
        for n in ["left", "down", "up", "right"] {
            assert!(scan_code(n).unwrap().1, "{n}");
        }
    }

    #[test]
    fn hotkeys_parse() {
        assert_eq!(parse_hotkey("F8"), Some((0, 0x77)));
        assert_eq!(parse_hotkey("f12"), Some((0, 0x7B)));
        assert_eq!(
            parse_hotkey("Ctrl+Shift+P"),
            Some((MOD_CONTROL | MOD_SHIFT, 0x50))
        );
        assert_eq!(parse_hotkey("alt + 5"), Some((MOD_ALT, 0x35)));
        assert_eq!(parse_hotkey("F25"), None);
        assert_eq!(parse_hotkey("ctrl"), None);
        assert_eq!(parse_hotkey("F8+F9"), None);
        assert_eq!(parse_hotkey(""), None);
    }

    #[test]
    fn exe_match() {
        let c = [
            "org.vinegarhq.Sober".to_string(),
            "RobloxPlayerBeta.exe".to_string(),
        ];
        assert!(exe_matches(
            r"C:\Users\x\AppData\Local\Roblox\RobloxPlayerBeta.exe",
            &c
        ));
        assert!(exe_matches(r"c:\robloxplayerbeta.EXE", &c));
        assert!(!exe_matches(r"C:\Windows\explorer.exe", &c));
        assert!(!exe_matches("", &c));
    }
}
