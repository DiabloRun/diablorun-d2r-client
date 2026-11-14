//! Tray icon, menu, and Explorer-restart handling.

use std::mem::size_of;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, LoadIconW, PostMessageW,
    SetForegroundWindow, TrackPopupMenu, HICON, MF_SEPARATOR, MF_STRING, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, WM_NULL,
};

pub const TRAY_CALLBACK_MESSAGE: u32 = 0x0400 + 1;

pub const MENU_SETTINGS: usize = 102;
pub const MENU_QUIT: usize = 105;

const TRAY_TITLE: &str = "D2R LRT Tracker";

pub struct TrayIcon {
    hwnd: HWND,
    icon: HICON,
    added: bool,
    taskbar_created_message: u32,
}

impl TrayIcon {
    pub fn new(hwnd: HWND) -> Self {
        let icon = unsafe {
            let module = GetModuleHandleW(None).unwrap_or_default();
            let instance = HINSTANCE(module.0);
            LoadIconW(Some(instance), PCWSTR(1 as *const u16))
                .or_else(|_| LoadIconW(None, PCWSTR(32512 as *const u16))) // IDI_APPLICATION
                .unwrap_or_default()
        };
        let taskbar_created_message = unsafe {
            windows::Win32::UI::WindowsAndMessaging::RegisterWindowMessageW(w!("TaskbarCreated"))
        };

        Self {
            hwnd,
            icon,
            added: false,
            taskbar_created_message,
        }
    }

    pub fn taskbar_created_message(&self) -> u32 {
        self.taskbar_created_message
    }

    pub fn add(&mut self) {
        let data = self.data();
        self.added = unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool();
    }

    pub fn recreate(&mut self) {
        self.added = false;
        self.add();
    }

    pub fn remove(&mut self) {
        if self.added {
            let data = self.data();
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            }
            self.added = false;
        }
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: TRAY_CALLBACK_MESSAGE,
            hIcon: self.icon,
            ..Default::default()
        };

        let encoded: Vec<u16> = TRAY_TITLE.encode_utf16().take(127).collect();
        data.szTip[..encoded.len()].copy_from_slice(&encoded);
        data
    }

    /// Show the tray menu at the cursor. Returns the chosen command id, or 0.
    pub fn show_menu(&self) -> usize {
        unsafe {
            let menu = match CreatePopupMenu() {
                Ok(menu) => menu,
                Err(_) => return 0,
            };

            let _ = AppendMenuW(menu, MF_STRING, MENU_SETTINGS, w!("Settings..."));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(menu, MF_STRING, MENU_QUIT, w!("Quit"));

            let mut point = POINT::default();
            if GetCursorPos(&mut point).is_err() {
                let _ = DestroyMenu(menu);
                return 0;
            }

            let _ = SetForegroundWindow(self.hwnd);
            let command = TrackPopupMenu(
                menu,
                TPM_RIGHTBUTTON | TPM_RETURNCMD,
                point.x,
                point.y,
                None,
                self.hwnd,
                None,
            )
            .0 as usize;
            let _ = PostMessageW(Some(self.hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu);
            command
        }
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        self.remove();
    }
}
