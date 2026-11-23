//! Win32 glue: desktop-layer windows, monitors, autostart, dark menus.

use windows::{
    core::{BOOL, PCSTR, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, RECT},
        Graphics::Gdi::{
            EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, HDC, HMONITOR, MONITORINFO,
            MONITOR_DEFAULTTOPRIMARY,
        },
        System::{
            LibraryLoader::{GetProcAddress, LoadLibraryW},
            Registry::*,
        },
        UI::{
            HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
            WindowsAndMessaging::{FindWindowW, SetWindowLongPtrW, GWLP_HWNDPARENT},
        },
    },
};

use crate::wide;

/// Owned by the desktop window: hidden from Alt+Tab and the taskbar,
/// and "Show desktop" (Win+D) does not hide widgets.
pub fn pin_to_desktop(h: HWND) {
    unsafe {
        let class = wide("Progman");
        if let Ok(progman) = FindWindowW(PCWSTR(class.as_ptr()), PCWSTR::null()) {
            SetWindowLongPtrW(h, GWLP_HWNDPARENT, progman.0 as isize);
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Monitor {
    pub rect: RECT,
    pub work: RECT,
    pub scale: f32,
}

fn info(h: HMONITOR) -> Option<Monitor> {
    unsafe {
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(h, &mut mi).as_bool() {
            return None;
        }
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(h, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        Some(Monitor { rect: mi.rcMonitor, work: mi.rcWork, scale: dx as f32 / 96.0 })
    }
}

pub fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn cb(h: HMONITOR, _: HDC, _: *mut RECT, l: LPARAM) -> BOOL {
        let v = &mut *(l.0 as *mut Vec<Monitor>);
        if let Some(m) = info(h) {
            v.push(m);
        }
        BOOL(1)
    }
    let mut v: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut v as *mut _ as isize));
    }
    v
}

pub fn primary() -> Monitor {
    unsafe { info(MonitorFromPoint(Default::default(), MONITOR_DEFAULTTOPRIMARY)) }.unwrap_or(Monitor {
        rect: RECT { left: 0, top: 0, right: 1920, bottom: 1080 },
        work: RECT { left: 0, top: 0, right: 1920, bottom: 1040 },
        scale: 1.0,
    })
}

/// Monitor containing the centre of `r` (nearest one otherwise).
pub fn monitor_for(r: &RECT) -> Monitor {
    let (cx, cy) = ((r.left + r.right) / 2, (r.top + r.bottom) / 2);
    let all = monitors();
    let dist = |m: &Monitor| {
        let dx = (m.rect.left - cx).max(0).max(cx - m.rect.right + 1);
        let dy = (m.rect.top - cy).max(0).max(cy - m.rect.bottom + 1);
        dx as i64 * dx as i64 + dy as i64 * dy as i64
    };
    all.into_iter().min_by_key(dist).unwrap_or_else(primary)
}

// ---------- autostart (HKCU\…\Run, value "Win11Widget" — same as the old version) ----------

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "Win11Widget";

pub fn autostart_enabled() -> bool {
    let (k, v) = (wide(RUN_KEY), wide(RUN_VALUE));
    unsafe {
        RegGetValueW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()), PCWSTR(v.as_ptr()), RRF_RT_REG_SZ, None, None, None).is_ok()
    }
}

pub fn set_autostart(on: bool) {
    let (k, v) = (wide(RUN_KEY), wide(RUN_VALUE));
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()), None, KEY_SET_VALUE, &mut key).is_err() {
            return;
        }
        if on {
            let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            let data = wide(&format!("\"{exe}\""));
            let bytes = std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2);
            let _ = RegSetValueExW(key, PCWSTR(v.as_ptr()), None, REG_SZ, Some(bytes));
        } else {
            let _ = RegDeleteValueW(key, PCWSTR(v.as_ptr()));
        }
        let _ = RegCloseKey(key);
    }
}

// ---------- dark context menus (uxtheme's undocumented but stable SetPreferredAppMode) ----------

pub fn dark_menus() {
    unsafe {
        let Ok(ux) = LoadLibraryW(PCWSTR(wide("uxtheme.dll").as_ptr())) else { return };
        // ordinal 135: SetPreferredAppMode(ForceDark = 2); 136: FlushMenuThemes
        if let Some(f) = GetProcAddress(ux, PCSTR(135 as *const u8)) {
            let f: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(f);
            f(2);
        }
        if let Some(f) = GetProcAddress(ux, PCSTR(136 as *const u8)) {
            let f: unsafe extern "system" fn() = std::mem::transmute(f);
            f();
        }
    }
}
