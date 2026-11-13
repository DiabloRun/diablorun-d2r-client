//! Global hotkey parsing and registration.

#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hotkey {
    /// `MOD_*` flags.
    pub modifiers: u32,
    /// Virtual-key code.
    pub virtual_key: u16,
}

pub fn parse_hotkey(input: &str) -> Result<Hotkey, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("hotkey is empty".into());
    }

    let parts: Vec<&str> = trimmed.split('+').map(str::trim).collect();
    if parts.iter().any(|part| part.is_empty()) {
        return Err("hotkey has an empty part".into());
    }

    let mut modifiers = 0u32;
    let mut key = None;

    for part in parts {
        let lower = part.to_ascii_lowercase();
        let modifier = match lower.as_str() {
            "ctrl" | "control" => Some(mod_flag("control")),
            "alt" => Some(mod_flag("alt")),
            "shift" => Some(mod_flag("shift")),
            "win" | "windows" | "super" | "meta" => Some(mod_flag("win")),
            _ => None,
        };

        if let Some(modifier) = modifier {
            if key.is_some() {
                return Err("modifiers must come before the key".into());
            }
            modifiers |= modifier;
        } else if key.is_none() {
            key = Some(virtual_key(part)?);
        } else {
            return Err(format!("unexpected key '{part}'"));
        }
    }

    let Some(virtual_key) = key else {
        return Err("hotkey is missing a key".into());
    };

    Ok(Hotkey {
        modifiers,
        virtual_key,
    })
}

#[cfg(windows)]
fn mod_flag(name: &str) -> u32 {
    match name {
        "control" => MOD_CONTROL.0,
        "alt" => MOD_ALT.0,
        "shift" => MOD_SHIFT.0,
        "win" => MOD_WIN.0,
        _ => 0,
    }
}

#[cfg(not(windows))]
fn mod_flag(_name: &str) -> u32 {
    0
}

fn virtual_key(name: &str) -> Result<u16, String> {
    let upper = name.to_ascii_uppercase();

    if upper.len() == 1 {
        let byte = upper.as_bytes()[0];
        if byte.is_ascii_uppercase() || byte.is_ascii_digit() {
            return Ok(u16::from(byte));
        }
    }

    if let Some(number) = upper.strip_prefix('F') {
        if let Ok(number) = number.parse::<u16>() {
            if (1..=24).contains(&number) {
                #[cfg(windows)]
                {
                    return Ok(vk_f(number));
                }
                #[cfg(not(windows))]
                {
                    return Ok(0x70 + number - 1);
                }
            }
        }
    }

    match upper.as_str() {
        "SPACE" => Ok(0x20),
        "TAB" => Ok(0x09),
        "ENTER" | "RETURN" => Ok(0x0D),
        "ESC" | "ESCAPE" => Ok(0x1B),
        "BACKSPACE" => Ok(0x08),
        "INSERT" => Ok(0x2D),
        "DELETE" | "DEL" => Ok(0x2E),
        "HOME" => Ok(0x24),
        "END" => Ok(0x23),
        "PAGEUP" | "PRIOR" => Ok(0x21),
        "PAGEDOWN" | "NEXT" => Ok(0x22),
        "LEFT" => Ok(0x25),
        "UP" => Ok(0x26),
        "RIGHT" => Ok(0x27),
        "DOWN" => Ok(0x28),
        "SCROLLLOCK" => Ok(0x91),
        "PAUSE" => Ok(0x13),
        "CAPSLOCK" => Ok(0x14),
        "NUMLOCK" => Ok(0x90),
        _ => Err(format!("unknown key '{name}'")),
    }
}

#[cfg(windows)]
fn vk_f(number: u16) -> u16 {
    match number {
        1 => 0x70,
        2 => 0x71,
        3 => 0x72,
        4 => 0x73,
        5 => 0x74,
        6 => 0x75,
        7 => 0x76,
        8 => 0x77,
        9 => 0x78,
        10 => 0x79,
        11 => 0x7A,
        12 => 0x7B,
        13 => 0x7C,
        14 => 0x7D,
        15 => 0x7E,
        16 => 0x7F,
        17 => 0x80,
        18 => 0x81,
        19 => 0x82,
        20 => 0x83,
        21 => 0x84,
        22 => 0x85,
        23 => 0x86,
        24 => 0x87,
        _ => 0,
    }
}

#[cfg(windows)]
pub fn register(
    hwnd: windows::Win32::Foundation::HWND,
    id: i32,
    hotkey: Hotkey,
) -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::RegisterHotKey;

    unsafe {
        RegisterHotKey(
            Some(hwnd),
            id,
            HOT_KEY_MODIFIERS(hotkey.modifiers | 0x4000), // MOD_NOREPEAT
            u32::from(hotkey.virtual_key),
        )
        .map_err(|error| error.message())
    }
}

#[cfg(windows)]
pub fn unregister(hwnd: windows::Win32::Foundation::HWND, id: i32) {
    use windows::Win32::UI::Input::KeyboardAndMouse::UnregisterHotKey;

    unsafe {
        let _ = UnregisterHotKey(Some(hwnd), id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_hotkey() {
        let hotkey = parse_hotkey("Ctrl+Alt+L").expect("parse");
        assert_ne!(hotkey.modifiers, 0);
        assert_eq!(hotkey.virtual_key, b'L' as u16);
    }

    #[test]
    fn parses_function_keys_and_aliases() {
        assert!(parse_hotkey("Shift+F12").is_ok());
        assert!(parse_hotkey("Win+Space").is_ok());
        assert!(parse_hotkey("control + shift + alt + k").is_ok());
    }

    #[test]
    fn rejects_invalid_combinations() {
        assert!(parse_hotkey("").is_err());
        assert!(parse_hotkey("Ctrl+").is_err());
        assert!(parse_hotkey("Ctrl+Nope").is_err());
        assert!(parse_hotkey("L+Ctrl").is_err());
        assert!(parse_hotkey("F25").is_err());
    }
}
