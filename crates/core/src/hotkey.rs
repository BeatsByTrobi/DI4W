//! Converts between "Ctrl+Alt+I" and the modifier flags / virtual-key code `RegisterHotKey` takes.

pub const MOD_ALT: u32 = 0x1;
pub const MOD_CONTROL: u32 = 0x2;
pub const MOD_SHIFT: u32 = 0x4;
pub const MOD_WIN: u32 = 0x8;

const MODIFIERS: [(&str, u32); 4] = [("Ctrl", MOD_CONTROL), ("Alt", MOD_ALT), ("Shift", MOD_SHIFT), ("Win", MOD_WIN)];

const KEY_NAMES: [(u32, &str); 26] = [
    (0x20, "Space"), (0x0D, "Enter"), (0x09, "Tab"), (0x08, "Backspace"), (0x2E, "Delete"), (0x2D, "Insert"),
    (0x24, "Home"), (0x23, "End"), (0x21, "PageUp"), (0x22, "PageDown"),
    (0x25, "Left"), (0x26, "Up"), (0x27, "Right"), (0x28, "Down"), (0x13, "Pause"),
    (0xBA, ";"), (0xBB, "="), (0xBC, ","), (0xBD, "-"), (0xBE, "."), (0xBF, "/"), (0xC0, "`"),
    (0xDB, "["), (0xDC, "\\"), (0xDD, "]"), (0xDE, "'"),
];

/// `(modifiers, key)` for a shortcut text, or `None` when it has no modifier, no key, or two keys.
pub fn parse(text: &str) -> Option<(u32, u32)> {
    let mut modifiers = 0;
    let mut key = 0;
    for raw in text.split('+').map(str::trim).filter(|s| !s.is_empty()) {
        if let Some((_, flag)) = MODIFIERS.iter().find(|(name, _)| name.eq_ignore_ascii_case(raw)) {
            modifiers |= flag;
            continue;
        }
        if key != 0 {
            return None; // two non-modifier keys
        }
        key = parse_key(raw)?;
    }
    (key != 0 && modifiers != 0).then_some((modifiers, key))
}

/// "Ctrl+Alt+Shift+Win+K": modifiers always in this order.
pub fn format(modifiers: u32, key: u32) -> String {
    let mut parts: Vec<String> =
        MODIFIERS.iter().filter(|(_, flag)| modifiers & flag != 0).map(|(name, _)| name.to_string()).collect();
    parts.push(key_name(key));
    parts.join("+")
}

/// Display name of a virtual-key code (unknown keys show as hex, which [`parse`] reads back).
pub fn key_name(key: u32) -> String {
    match key {
        0x30..=0x39 | 0x41..=0x5A => char::from_u32(key).unwrap_or('?').to_string(),
        0x60..=0x69 => format!("Num{}", key - 0x60),
        0x70..=0x87 => format!("F{}", key - 0x6F),
        _ => KEY_NAMES
            .iter()
            .find(|(vk, _)| *vk == key)
            .map(|(_, name)| name.to_string())
            .unwrap_or_else(|| format!("0x{key:02X}")),
    }
}

fn parse_key(name: &str) -> Option<u32> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next())
        && c.is_ascii_alphanumeric() {
            return Some(c.to_ascii_uppercase() as u32);
        }
    let lower = name.to_ascii_lowercase();
    if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()).filter(|n| (1..=24).contains(n)) {
        return Some(0x6F + n);
    }
    if let Some(n) = lower.strip_prefix("num").and_then(|n| n.parse::<u32>().ok()).filter(|n| *n <= 9) {
        return Some(0x60 + n);
    }
    if let Some(hex) = lower.strip_prefix("0x") {
        return u32::from_str_radix(hex, 16).ok().filter(|k| *k != 0);
    }
    KEY_NAMES.iter().find(|(_, n)| n.eq_ignore_ascii_case(name)).map(|(vk, _)| *vk)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_shortcuts() {
        assert_eq!(parse("Ctrl+Alt+I"), Some((MOD_CONTROL | MOD_ALT, 0x49)));
        assert_eq!(parse("ctrl + shift + F5"), Some((MOD_CONTROL | MOD_SHIFT, 0x74)));
        assert_eq!(parse("Win+Num1"), Some((MOD_WIN, 0x61)));
        assert_eq!(parse("Alt+Space"), Some((MOD_ALT, 0x20)));
        assert_eq!(parse("Ctrl+0x7B"), Some((MOD_CONTROL, 0x7B)));
    }

    #[test]
    fn rejects_invalid_shortcuts() {
        for text in ["", "  ", "I", "Ctrl+Alt", "Ctrl+A+B", "Ctrl+NotAKey"] {
            assert_eq!(parse(text), None, "{text}");
        }
    }

    #[test]
    fn format_and_parse_round_trip() {
        for text in ["Ctrl+Alt+I", "Ctrl+Shift+F12", "Win+Num7", "Alt+Space", "Ctrl+0xE2"] {
            let (m, k) = parse(text).unwrap();
            assert_eq!(parse(&format(m, k)), Some((m, k)), "{text}");
        }
    }

    #[test]
    fn formats_modifiers_in_a_fixed_order() {
        assert_eq!(format(MOD_WIN | MOD_SHIFT | MOD_ALT | MOD_CONTROL, 0x4B), "Ctrl+Alt+Shift+Win+K");
    }
}
