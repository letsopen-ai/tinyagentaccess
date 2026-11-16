//! Portable vocabulary represented by X11-compatible keysyms (also used by portals).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key(pub u32);
pub type KeyCombo = Vec<Key>;
impl Key {
    pub fn modifier(self) -> bool {
        matches!(self.0, 0xffe1 | 0xffe3 | 0xffe9 | 0xffeb | 0x11000000)
    }
    pub fn parse(name: &str) -> Result<Self, String> {
        if name.is_empty() {
            return Err("empty key segment in combination".into());
        }
        let lower = name.to_ascii_lowercase();
        let sym = match lower.as_str() {
            "ctrl" | "control" => 0xffe3,
            "shift" => 0xffe1,
            "alt" | "option" => 0xffe9,
            "cmd" | "command" | "super" | "meta" | "win" | "windows" => 0xffeb,
            "fn" => 0x11000000,
            "enter" | "return" => 0xff0d,
            "tab" => 0xff09,
            "esc" | "escape" => 0xff1b,
            "space" => 32,
            "backspace" => 0xff08,
            "delete" | "del" => 0xffff,
            "insert" => 0xff63,
            "home" => 0xff50,
            "end" => 0xff57,
            "pageup" | "page_up" | "prior" => 0xff55,
            "pagedown" | "page_down" | "next" => 0xff56,
            "left" | "arrowleft" => 0xff51,
            "up" | "arrowup" => 0xff52,
            "right" | "arrowright" => 0xff53,
            "down" | "arrowdown" => 0xff54,
            "capslock" | "caps_lock" => 0xffe5,
            "numlock" | "num_lock" => 0xff7f,
            "scrolllock" | "scroll_lock" => 0xff14,
            "print" | "printscreen" => 0xff61,
            "pause" => 0xff13,
            "menu" => 0xff67,
            "plus" => 43,
            "minus" => 45,
            "equal" => 61,
            "slash" => 47,
            "volumeup" | "xf86audioraisevolume" => 0x1008ff13,
            "volumedown" | "xf86audiolowervolume" => 0x1008ff11,
            "mute" | "xf86audiomute" => 0x1008ff12,
            "play" | "xf86audioplay" => 0x1008ff14,
            "kp_space" => 0xff80,
            "kp_tab" => 0xff89,
            "kp_f1" => 0xff91,
            "kp_f2" => 0xff92,
            "kp_f3" => 0xff93,
            "kp_f4" => 0xff94,
            "kp_separator" => 0xffac,
            "kp_enter" => 0xff8d,
            "kp_add" => 0xffab,
            "kp_subtract" => 0xffad,
            "kp_multiply" => 0xffaa,
            "kp_divide" => 0xffaf,
            "kp_decimal" => 0xffae,
            "kp_equal" => 0xffbd,
            "kp_home" => 0xff95,
            "kp_left" => 0xff96,
            "kp_up" => 0xff97,
            "kp_right" => 0xff98,
            "kp_down" => 0xff99,
            "kp_prior" | "kp_page_up" => 0xff9a,
            "kp_next" | "kp_page_down" => 0xff9b,
            "kp_end" => 0xff9c,
            "kp_begin" => 0xff9d,
            "kp_insert" => 0xff9e,
            "kp_delete" => 0xff9f,
            _ => {
                if let Some(n) = lower
                    .strip_prefix('f')
                    .and_then(|s| s.parse::<u32>().ok())
                    .filter(|n| (1..=24).contains(n))
                {
                    0xffbd + n
                } else if let Some(n) = lower
                    .strip_prefix("kp_")
                    .and_then(|s| s.parse::<u32>().ok())
                    .filter(|n| *n <= 9)
                {
                    0xffb0 + n
                } else if name.chars().count() == 1 && !name.chars().next().unwrap().is_control() {
                    return Ok(Self::character(name.chars().next().unwrap()));
                } else {
                    return Err(format!("unknown key {name}"));
                }
            }
        };
        Ok(Self(sym))
    }
    pub fn character(c: char) -> Self {
        Self(if (c as u32) <= 255 {
            c as u32
        } else {
            0x01000000 | c as u32
        })
    }
}
pub fn combo(s: &str, modifiers_only: bool) -> Result<KeyCombo, String> {
    if s.is_empty() {
        return if modifiers_only {
            Ok(vec![])
        } else {
            Err("empty key combination".into())
        };
    }
    let mut parts: Vec<_> = s.split('+').collect();
    if s.ends_with('+') {
        parts.pop();
        if parts.last() == Some(&"") {
            parts.pop();
        }
        parts.push("plus");
    }
    let mut out = vec![];
    for p in parts {
        let k = Key::parse(p)?;
        if modifiers_only && !k.modifier() {
            return Err(format!("key {p} is not a modifier"));
        }
        if !out.contains(&k) {
            out.push(k);
        }
    }
    Ok(out)
}
/// Canonical named vocabulary, shared by backend mapping audits.
pub const SPECIAL_NAMES: &[&str] = &[
    "ctrl",
    "shift",
    "alt",
    "super",
    "fn",
    "enter",
    "tab",
    "esc",
    "space",
    "backspace",
    "delete",
    "insert",
    "home",
    "end",
    "pageup",
    "pagedown",
    "left",
    "up",
    "right",
    "down",
    "capslock",
    "numlock",
    "scrolllock",
    "printscreen",
    "pause",
    "menu",
    "volumeup",
    "volumedown",
    "mute",
    "play",
    "kp_space",
    "kp_tab",
    "kp_f1",
    "kp_f2",
    "kp_f3",
    "kp_f4",
    "kp_separator",
    "kp_enter",
    "kp_add",
    "kp_subtract",
    "kp_multiply",
    "kp_divide",
    "kp_decimal",
    "kp_equal",
    "kp_home",
    "kp_left",
    "kp_up",
    "kp_right",
    "kp_down",
    "kp_prior",
    "kp_next",
    "kp_end",
    "kp_begin",
    "kp_insert",
    "kp_delete",
];
/// Human-readable names for unavailable special mappings.
pub fn label(k: Key) -> String {
    if let Some(name) = SPECIAL_NAMES
        .iter()
        .find(|name| Key::parse(name).ok() == Some(k))
    {
        return (*name).into();
    }
    if (0xffbe..=0xffd5).contains(&k.0) {
        return format!("F{}", k.0 - 0xffbd);
    }
    format!("keysym {:#x}", k.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vocabulary() {
        for (a, b) in [
            ("Return", "enter"),
            ("ALT", "option"),
            ("super", "windows"),
            ("Prior", "page_up"),
            ("BackSpace", "backspace"),
        ] {
            assert_eq!(Key::parse(a), Key::parse(b));
        }
        assert_eq!(combo("ctrl++", false), combo("ctrl+plus", false));
        assert_eq!(combo("+", false).unwrap(), vec![Key(43)]);
        assert!(combo("ctrl+bogus", false).unwrap_err().contains("bogus"));
        assert!(combo("a", true).is_err());
        for s in ["ctrl++a", "++"] {
            assert!(combo(s, false).unwrap_err().contains("empty key segment"));
        }
        assert_ne!(Key::parse("fn").unwrap(), Key(0x1008ff2b));
        for i in 1..=24 {
            assert_eq!(Key::parse(&format!("F{i}")).unwrap(), Key(0xffbd + i));
        }
    }
}
