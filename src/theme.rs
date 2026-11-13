//! Follows the Windows light/dark app theme for the native settings dialog.

use std::mem::{size_of, transmute};
use std::sync::OnceLock;

use windows::core::{w, BOOL, PCSTR, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
use windows::Win32::Graphics::Gdi::{
    GetStockObject, InvalidateRect, SetBkColor, SetBkMode, SetDCBrushColor, SetTextColor,
    UpdateWindow, DC_BRUSH, HDC, OPAQUE, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::UI::Controls::SetWindowTheme;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, GetClassNameW, SendMessageW, WM_THEMECHANGED,
};

const DARK_BACKGROUND: COLORREF = COLORREF(0x0020_2020);
const DARK_TEXT: COLORREF = COLORREF(0x00F0_F0F0);

const WM_CTLCOLOREDIT: u32 = 0x0133;
const WM_CTLCOLORLISTBOX: u32 = 0x0134;
const WM_CTLCOLORBTN: u32 = 0x0135;
const WM_CTLCOLORDLG: u32 = 0x0136;
const WM_CTLCOLORSTATIC: u32 = 0x0138;

const DWMWA_USE_IMMERSIVE_DARK_MODE: i32 = 20;
const DWMWA_USE_IMMERSIVE_DARK_MODE_BEFORE_20H1: i32 = 19;

#[repr(i32)]
enum PreferredAppMode {
    #[allow(dead_code)]
    Default = 0,
    AllowDark = 1,
    #[allow(dead_code)]
    ForceDark = 2,
    #[allow(dead_code)]
    ForceLight = 3,
    #[allow(dead_code)]
    Max = 4,
}

type ShouldAppsUseDarkModeFn = unsafe extern "system" fn() -> bool;
type AllowDarkModeForWindowFn = unsafe extern "system" fn(HWND, bool) -> bool;
type SetPreferredAppModeFn = unsafe extern "system" fn(PreferredAppMode) -> PreferredAppMode;
type FlushMenuThemesFn = unsafe extern "system" fn();

struct Api {
    should_use_dark_mode: ShouldAppsUseDarkModeFn,
    allow_dark_mode_for_window: AllowDarkModeForWindowFn,
    set_preferred_app_mode: SetPreferredAppModeFn,
    flush_menu_themes: FlushMenuThemesFn,
}

/// Undocumented uxtheme exports used by Explorer and most dark-mode Win32 apps.
fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(|| unsafe {
        let module = GetModuleHandleW(w!("uxtheme.dll"))
            .or_else(|_| LoadLibraryW(w!("uxtheme.dll")))
            .ok()?;
        let should_use_dark_mode = GetProcAddress(module, PCSTR(132usize as *const u8))?;
        let allow_dark_mode_for_window = GetProcAddress(module, PCSTR(133usize as *const u8))?;
        let set_preferred_app_mode = GetProcAddress(module, PCSTR(135usize as *const u8))?;
        let flush_menu_themes = GetProcAddress(module, PCSTR(136usize as *const u8))?;

        Some(Api {
            should_use_dark_mode: transmute(should_use_dark_mode),
            allow_dark_mode_for_window: transmute(allow_dark_mode_for_window),
            set_preferred_app_mode: transmute(set_preferred_app_mode),
            flush_menu_themes: transmute(flush_menu_themes),
        })
    })
    .as_ref()
}

/// Opt the process into Windows' dark mode so menus and controls follow the
/// user's app theme.
pub fn init() {
    let Some(api) = api() else {
        return;
    };
    unsafe {
        (api.set_preferred_app_mode)(PreferredAppMode::AllowDark);
        (api.flush_menu_themes)();
    }
}

pub fn is_dark() -> bool {
    api().is_some_and(|api| unsafe { (api.should_use_dark_mode)() })
}

/// Apply the current theme to a dialog and all of its child controls.
pub fn apply(hwnd: HWND, dark: bool) {
    let Some(api) = api() else {
        return;
    };
    unsafe {
        (api.allow_dark_mode_for_window)(hwnd, dark);
        set_titlebar(hwnd, dark);
        let _ = EnumChildWindows(Some(hwnd), Some(apply_to_child), LPARAM(dark as isize));
        let _ = InvalidateRect(Some(hwnd), None, true);
        let _ = UpdateWindow(hwnd);
    }
}

unsafe extern "system" fn apply_to_child(child: HWND, lparam: LPARAM) -> BOOL {
    let dark = lparam.0 != 0;
    if let Some(api) = api() {
        unsafe {
            (api.allow_dark_mode_for_window)(child, dark);
        }
    }

    let mut buffer = [0u16; 32];
    let length = unsafe { GetClassNameW(child, &mut buffer) }.max(0) as usize;
    let class = String::from_utf16_lossy(&buffer[..length]);
    let theme = if dark {
        match class.as_str() {
            "Button" => w!("DarkMode_Explorer"),
            "Edit" | "ComboBox" => w!("DarkMode_CFD"),
            _ => PCWSTR::null(),
        }
    } else {
        PCWSTR::null()
    };
    unsafe {
        let _ = SetWindowTheme(child, theme, PCWSTR::null());
    }
    unsafe {
        let _ = SendMessageW(child, WM_THEMECHANGED, None, None);
    }
    BOOL(1)
}

fn set_titlebar(hwnd: HWND, dark: bool) {
    let value = BOOL::from(dark);
    let result = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWINDOWATTRIBUTE(DWMWA_USE_IMMERSIVE_DARK_MODE),
            &value as *const _ as *const _,
            size_of::<BOOL>() as u32,
        )
    };
    if result.is_err() {
        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWINDOWATTRIBUTE(DWMWA_USE_IMMERSIVE_DARK_MODE_BEFORE_20H1),
                &value as *const _ as *const _,
                size_of::<BOOL>() as u32,
            );
        }
    }
}

/// Return the colors and background brush for a control-color message.
pub fn control_color(message: u32, wparam: WPARAM) -> Option<isize> {
    if !is_dark() {
        return None;
    }
    let hdc = HDC(wparam.0 as *mut _);
    unsafe {
        SetTextColor(hdc, DARK_TEXT);
        SetBkColor(hdc, DARK_BACKGROUND);
        if message == WM_CTLCOLORSTATIC {
            SetBkMode(hdc, TRANSPARENT);
        } else {
            SetBkMode(hdc, OPAQUE);
        }
        SetDCBrushColor(hdc, DARK_BACKGROUND);
        Some(GetStockObject(DC_BRUSH).0 as isize)
    }
}

pub fn is_control_color_message(message: u32) -> bool {
    matches!(
        message,
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN | WM_CTLCOLORDLG | WM_CTLCOLORSTATIC
    )
}
