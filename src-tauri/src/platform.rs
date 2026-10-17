//! Win32 glue: pin widgets to the desktop layer, wallpaper path.

use std::path::PathBuf;

use tauri::WebviewWindow;
use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{
        FindWindowW, SetWindowLongPtrW, SystemParametersInfoW, GWLP_HWNDPARENT, SPI_GETDESKWALLPAPER,
    },
};

pub fn setup_widget(w: &WebviewWindow) {
    let Ok(h) = w.hwnd() else { return };
    let h = h.0 as HWND;
    unsafe {
        // Owned by the desktop window: hidden from Alt+Tab and the taskbar,
        // and "Show desktop" (Win+D) does not hide widgets.
        let class: Vec<u16> = "Progman\0".encode_utf16().collect();
        let progman = FindWindowW(class.as_ptr(), std::ptr::null());
        if !progman.is_null() {
            SetWindowLongPtrW(h, GWLP_HWNDPARENT, progman as isize);
        }
    }
}

/// The current desktop wallpaper image file, if there is one.
pub fn wallpaper_path() -> Option<PathBuf> {
    let mut buf = [0u16; 1024];
    let ok = unsafe { SystemParametersInfoW(SPI_GETDESKWALLPAPER, buf.len() as u32, buf.as_mut_ptr() as _, 0) };
    if ok != 0 {
        let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
        let p = PathBuf::from(String::from_utf16_lossy(&buf[..len]));
        if len > 0 && p.is_file() {
            return Some(p);
        }
    }
    // Spotlight / slideshow wallpapers are only available as the transcoded copy.
    let p = PathBuf::from(std::env::var("APPDATA").ok()?).join(r"Microsoft\Windows\Themes\TranscodedWallpaper");
    p.is_file().then_some(p)
}
