//! Optional autostart entry under HKCU.

use windows::core::w;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};

pub fn is_enabled() -> bool {
    let mut buffer = [0u16; 1024];
    let mut size = (buffer.len() * 2) as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Run"),
            w!("D2R Load Remover"),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    };

    if result != ERROR_SUCCESS {
        return false;
    }

    let length = (size as usize / 2).saturating_sub(1);
    let value = String::from_utf16_lossy(&buffer[..length]);
    let executable = std::env::current_exe()
        .ok()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    value.contains(&executable)
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    if enabled {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        let command = format!("\"{}\"", executable.display());
        let wide: Vec<u16> = command.encode_utf16().chain([0]).collect();
        let bytes =
            unsafe { std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2) };

        let result = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                w!(r"Software\Microsoft\Windows\CurrentVersion\Run"),
                w!("D2R Load Remover"),
                REG_SZ.0,
                Some(bytes.as_ptr() as *const _),
                bytes.len() as u32,
            )
        };
        if result != ERROR_SUCCESS {
            return Err(format!("registry error {}", result.0));
        }
    } else {
        let result = unsafe {
            RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                w!(r"Software\Microsoft\Windows\CurrentVersion\Run"),
                w!("D2R Load Remover"),
            )
        };
        if result != ERROR_SUCCESS && result != ERROR_FILE_NOT_FOUND {
            return Err(format!("registry error {}", result.0));
        }
    }

    Ok(())
}

pub fn toggle() -> Result<bool, String> {
    let enabled = !is_enabled();
    set_enabled(enabled)?;
    Ok(enabled)
}
