//! D2R window discovery and physical-pixel geometry.

use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::path::Path;

use windows::core::BOOL;
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClientRect, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible,
};

use crate::config::CaptureConfig;

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub hwnd: HWND,
    pub client_width: u32,
    pub client_height: u32,
}

/// Find the largest visible, non-minimized D2R window. Matching by process
/// image name is preferred; an exact window-title match is the fallback.
pub fn find_d2r_window(config: &CaptureConfig) -> Option<WindowInfo> {
    let mut candidates: Vec<WindowInfo> = Vec::new();

    let lparam = LPARAM(&mut candidates as *mut Vec<WindowInfo> as isize);
    unsafe {
        let _ = EnumWindows(Some(enum_window), lparam);
    }

    candidates
        .into_iter()
        .filter(|window| {
            window.client_width >= config.min_width && window.client_height >= config.min_height
        })
        .max_by_key(|window| (window.client_width as u64) * (window.client_height as u64))
}

unsafe extern "system" fn enum_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let candidates = unsafe { &mut *(lparam.0 as *mut Vec<WindowInfo>) };

    if !unsafe { IsWindowVisible(hwnd) }.as_bool() || unsafe { IsIconic(hwnd) }.as_bool() {
        return BOOL(1);
    }

    let mut client = RECT::default();
    if unsafe { GetClientRect(hwnd, &mut client) }.is_err() {
        return BOOL(1);
    }

    let width = (client.right - client.left).max(0) as u32;
    let height = (client.bottom - client.top).max(0) as u32;
    if width == 0 || height == 0 {
        return BOOL(1);
    }

    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    let process_path = if pid != 0 {
        process_image_name(pid)
    } else {
        None
    };

    // The process check is the normal path. The title fallback keeps detection
    // working when process access is denied (for example under some anti-cheat
    // or sandbox configurations). Only read the title when it is needed.
    let matches_process = process_path
        .as_deref()
        .and_then(|path| Path::new(path).file_name())
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case("D2R.exe"))
        .unwrap_or(false);

    if matches_process || window_title(hwnd) == "Diablo II: Resurrected" {
        candidates.push(WindowInfo {
            hwnd,
            client_width: width,
            client_height: height,
        });
    }

    BOOL(1)
}

pub fn window_title(hwnd: HWND) -> String {
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    if length <= 0 {
        return String::new();
    }

    let mut buffer = vec![0u16; length as usize + 1];
    let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    if copied <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..copied as usize])
}

fn process_image_name(pid: u32) -> Option<String> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buffer = vec![0u16; 1024];
    let mut size = buffer.len() as u32;
    let result = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut size,
        )
    };
    unsafe {
        let _ = CloseHandle(handle);
    }
    result.ok()?;
    Some(String::from_utf16_lossy(&buffer[..size as usize]))
}

pub fn is_capturable(hwnd: HWND) -> bool {
    unsafe {
        IsWindow(Some(hwnd)).as_bool()
            && IsWindowVisible(hwnd).as_bool()
            && !IsIconic(hwnd).as_bool()
    }
}

pub fn client_size(hwnd: HWND) -> Option<(u32, u32)> {
    let mut rect = RECT::default();
    unsafe { GetClientRect(hwnd, &mut rect) }.ok()?;
    let width = (rect.right - rect.left).max(0) as u32;
    let height = (rect.bottom - rect.top).max(0) as u32;
    (width > 0 && height > 0).then_some((width, height))
}

/// Window rectangle in physical pixels, excluding DWM's invisible resize
/// borders when possible.
pub fn window_rect(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    let dwm_result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut RECT as *mut c_void,
            size_of::<RECT>() as u32,
        )
    };
    if dwm_result.is_ok() {
        return Some(rect);
    }

    let mut fallback = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut fallback) }.ok()?;
    Some(fallback)
}

pub fn monitor_for_window(hwnd: HWND) -> Option<(HMONITOR, RECT)> {
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_invalid() {
        return None;
    }

    let mut info: MONITORINFO = unsafe { zeroed() };
    info.cbSize = size_of::<MONITORINFO>() as u32;
    if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        Some((monitor, info.rcMonitor))
    } else {
        None
    }
}
