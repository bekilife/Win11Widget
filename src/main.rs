//! Win11Widget — macOS-style desktop widgets for Windows 11.
//! Native Win32: one process, layered windows drawn with Direct2D/DirectWrite.

#![windows_subsystem = "windows"]
#![allow(clippy::too_many_arguments, clippy::missing_transmute_annotations)]

mod cities;
mod gfx;
mod gpu;
mod http;
mod icons;
mod image;
mod json;
mod layout;
mod media;
mod platform;
mod rates;
mod settings;
mod system;
mod wallpaper;
mod weather;
mod widgets;

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    sync::mpsc::Sender,
};

use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::*,
        Graphics::{Direct2D::ID2D1Bitmap, Gdi::HBRUSH},
        System::{
            Com::{CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED},
            LibraryLoader::GetModuleHandleW,
            SystemInformation::GetLocalTime,
            Threading::CreateMutexW,
        },
        UI::{
            Controls::{LoadIconMetric, LIM_SMALL},
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT},
            Shell::*,
            WindowsAndMessaging::*,
        },
    },
};

use crate::{
    gfx::{Gfx, Surface},
    layout::Rect as PRect,
    settings::{City, Settings},
    widgets::{Data, Hit, Zones},
};

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Id {
    Rates,
    Media,
    Weather,
    System,
}

impl Id {
    pub const ALL: [Id; 4] = [Id::Rates, Id::Media, Id::Weather, Id::System];
    pub fn name(self) -> &'static str {
        match self {
            Id::Rates => "rates",
            Id::Media => "media",
            Id::Weather => "weather",
            Id::System => "system",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Id::Rates => "Курсы",
            Id::Media => "Сейчас играет",
            Id::Weather => "Погода",
            Id::System => "Система",
        }
    }
    /// Default logical size.
    pub fn size(self) -> (f64, f64) {
        match self {
            Id::Rates => (360.0, 170.0),
            Id::Media | Id::System => (360.0, 150.0),
            Id::Weather => (360.0, 300.0),
        }
    }
}

/// Default stacking: rows top-to-bottom, wrapping into a new column to the left.
const LAYOUT: &[&[Id]] = &[&[Id::Rates], &[Id::Media], &[Id::Weather], &[Id::System]];

// messages
const WM_TRAY: u32 = WM_APP + 1;
const WM_MEDIA: u32 = WM_APP + 2;
const WM_STATS: u32 = WM_APP + 3;
const WM_WEATHER: u32 = WM_APP + 4;
const WM_WALL: u32 = WM_APP + 5;
pub const WM_SEARCH: u32 = WM_APP + 6;
const WM_OPEN_CITIES: u32 = WM_APP + 7;
const WM_RERENDER: u32 = WM_APP + 8;
const WM_RATES: u32 = WM_APP + 9;
const WM_MOUSELEAVE: u32 = 0x02A3;

// timers (controller window)
const T_MINUTE: usize = 1;
const T_PROGRESS: usize = 2;
const T_WEATHER: usize = 3;
const T_SAVE: usize = 4;
const T_RATES: usize = 5;
const RATES_EVERY_MS: u32 = 60 * 1000;
// timers (system widget)
const T_OPEN: usize = 10;
const T_CLOSE: usize = 11;

const WEATHER_EVERY_MS: u32 = 15 * 60 * 1000;
const CTRL_CLASS: &str = "Win11Widget.Native.Ctrl";
const WIDGET_CLASS: &str = "Win11Widget.Native.Widget";

struct Drag {
    cursor: POINT,
    win: (i32, i32),
    moved: bool,
}

struct Widget {
    id: Id,
    hwnd: HWND,
    surf: Surface,
    zones: Zones,
    hover: Option<Hit>,
    pressed: Option<Hit>,
    drag: Option<Drag>,
    tracking: bool,
    /// Current logical height.
    h: f32,
    /// Current window rectangle, physical px.
    rect: PRect,
}

struct Expansion {
    /// Compact rectangle to return to.
    orig: PRect,
    /// Where we put the expanded window; if it is elsewhere on collapse, the user dragged it.
    at: (i32, i32),
}

pub struct App {
    pub gfx: Gfx,
    pub settings: Settings,
    widgets: Vec<Widget>,
    pub ctrl: HWND,
    weather: HashMap<String, weather::Forecast>,
    rates: rates::Rates,
    stats: Option<system::Stats>,
    media: media::MediaState,
    art: Option<ID2D1Bitmap>,
    art_gen: u32,
    media_tx: Sender<media::Cmd>,
    sys_wake: Sender<()>,
    walls: Vec<wallpaper::Wall>,
    glass: Vec<(RECT, ID2D1Bitmap)>,
    expansion: Option<Expansion>,
    pub cities: Option<cities::Editor>,
    taskbar_created: u32,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    /// The system widget while it is expanded (topmost instead of bottom-most).
    static EXPANDED: Cell<isize> = const { Cell::new(0) };
}

/// Run `f` with the app state. Returns None when the state is already borrowed (a message
/// re-entered from inside another handler) — such messages are simply skipped.
pub fn with<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

fn hinstance() -> HINSTANCE {
    unsafe { GetModuleHandleW(None).map(|m| m.into()).unwrap_or_default() }
}

pub fn post<T>(hwnd: HWND, msg: u32, payload: T) {
    let p = Box::into_raw(Box::new(payload));
    unsafe {
        if PostMessageW(Some(hwnd), msg, WPARAM(0), LPARAM(p as isize)).is_err() {
            drop(Box::from_raw(p));
        }
    }
}

unsafe fn take<T>(lp: LPARAM) -> Box<T> {
    Box::from_raw(lp.0 as *mut T)
}

fn unix_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
}

fn dpi_scale(hwnd: HWND) -> f32 {
    let d = unsafe { GetDpiForWindow(hwnd) };
    if d == 0 {
        1.0
    } else {
        d as f32 / 96.0
    }
}

fn work_area_of(r: PRect) -> (PRect, f32) {
    let m = platform::monitor_for(&RECT { left: r.x, top: r.y, right: r.right(), bottom: r.bottom() });
    (PRect::from_win(&m.work), m.scale)
}

fn on_screen(x: i32, y: i32) -> bool {
    platform::monitors().iter().any(|m| {
        let r = m.rect;
        x >= r.left - 40 && y >= r.top - 40 && x < r.right - 40 && y < r.bottom - 40
    })
}

impl App {
    fn idx(&self, hwnd: HWND) -> Option<usize> {
        self.widgets.iter().position(|w| w.hwnd == hwnd)
    }
    fn idx_of(&self, id: Id) -> Option<usize> {
        self.widgets.iter().position(|w| w.id == id)
    }

    fn data(&self, hover: Option<Hit>, pressed: Option<Hit>) -> Data<'_> {
        Data {
            unix_ms: unix_ms(),
            media: &self.media,
            art: self.art.as_ref(),
            cities: &self.settings.cities,
            weather: &self.weather,
            rates: &self.rates,
            stats: self.stats.as_ref(),
            hover,
            pressed,
            expanded: self.expansion.is_some(),
        }
    }

    /// Logical height the widget wants now.
    fn want_height(&self, id: Id) -> f32 {
        widgets::height(&self.gfx, id, &self.data(None, None))
    }

    fn glass_for(&mut self, r: PRect, scale: f32) -> Option<(ID2D1Bitmap, windows_numerics::Matrix3x2)> {
        let mon = platform::monitor_for(&RECT { left: r.x, top: r.y, right: r.right(), bottom: r.bottom() }).rect;
        let wall = self.walls.iter().find(|w| w.mon == mon)?;
        let bmp = match self.glass.iter().find(|(m, _)| *m == mon) {
            Some((_, b)) => b.clone(),
            None => {
                let b = self.gfx.bitmap(&wall.img)?;
                self.glass.push((mon, b.clone()));
                b
            }
        };
        let q = wall.img.w as f32 / (mon.right - mon.left).max(1) as f32;
        let k = 1.0 / (q * scale);
        let m = windows_numerics::Matrix3x2 {
            M11: k,
            M12: 0.0,
            M21: 0.0,
            M22: k,
            M31: -((r.x - mon.left) as f32) / scale,
            M32: -((r.y - mon.top) as f32) / scale,
        };
        Some((bmp, m))
    }

    /// Render widget `i` at its current rect and push it to the screen.
    fn paint(&mut self, i: usize) {
        let (hwnd, id, rect, hover, pressed) = {
            let w = &self.widgets[i];
            (w.hwnd, w.id, w.rect, w.hover, w.pressed)
        };
        let scale = dpi_scale(hwnd);
        let lw = id.size().0 as f32;
        let lh = self.widgets[i].h;
        let glass = self.glass_for(rect, scale);
        let mut zones = Vec::new();
        let mut surf = std::mem::replace(&mut self.widgets[i].surf, Surface::new());
        {
            let d = self.data(hover, pressed);
            self.gfx.draw(&mut surf, rect.w, rect.h, scale, |g| {
                widgets::card(g, lw, lh, glass.as_ref().map(|(b, m)| (b, *m)));
                widgets::render(g, id, &d, lw, &mut zones);
            });
        }
        surf.present(hwnd, Some((rect.x, rect.y)));
        #[cfg(debug_assertions)]
        if let Some(dir) = std::env::var_os("W11W_DUMP") {
            surf.dump(&std::path::Path::new(&dir).join(format!("{}.bmp", id.name())));
        }
        let w = &mut self.widgets[i];
        w.surf = surf;
        w.zones = zones;
        if !(id == Id::System && self.expansion.is_some()) {
            let s = self.settings.widget(id.name());
            if s.x != Some(rect.x) || s.y != Some(rect.y) {
                s.x = Some(rect.x);
                s.y = Some(rect.y);
                self.schedule_save();
            }
        }
    }

    fn paint_id(&mut self, id: Id) {
        if let Some(i) = self.idx_of(id) {
            self.paint(i);
        }
    }

    /// Move widget `i` (keeps its size).
    fn place(&mut self, i: usize, x: i32, y: i32) {
        self.widgets[i].rect.x = x;
        self.widgets[i].rect.y = y;
        self.paint(i);
    }

    fn rects(&self) -> Vec<(Id, PRect)> {
        self.widgets.iter().map(|w| (w.id, w.rect)).collect()
    }

    /// Re-measure a widget; if its height changed, resize it and move the widgets stacked
    /// under it by the same amount. Then repaint.
    fn update(&mut self, id: Id) {
        let Some(i) = self.idx_of(id) else { return };
        if id == Id::System && self.expansion.is_some() {
            self.expand(); // follows the content height while open
            return;
        }
        let h = self.want_height(id);
        let w = &self.widgets[i];
        if (h - w.h).abs() > 0.01 {
            let scale = dpi_scale(w.hwnd);
            let old = w.rect;
            let new_ph = (h * scale).round() as i32;
            self.widgets[i].h = h;
            self.widgets[i].rect.h = new_ph;
            let moves = layout::reflow_below(id, old, new_ph, &self.rects(), scale as f64);
            self.paint(i);
            for (mid, x, y) in moves {
                if let Some(j) = self.idx_of(mid) {
                    self.place(j, x, y);
                }
            }
        } else {
            self.paint(i);
        }
    }

    fn schedule_save(&self) {
        unsafe {
            SetTimer(Some(self.ctrl), T_SAVE, 400, None);
        }
    }

    fn default_positions(&self) -> Vec<(Id, i32, i32)> {
        let prim = platform::primary();
        let heights: HashMap<Id, f64> = Id::ALL.iter().map(|&id| (id, self.want_height(id) as f64)).collect();
        layout::default_positions(PRect::from_win(&prim.work), prim.scale as f64, LAYOUT, |id| {
            (id.size().0, heights[&id])
        })
    }

    fn create_widget(&mut self, id: Id) {
        if self.idx_of(id).is_some() {
            return;
        }
        let saved = self.settings.get(id.name()).and_then(|s| s.x.zip(s.y));
        let (x, y) = match saved {
            Some((x, y)) if on_screen(x, y) => (x, y),
            _ => self.default_positions().into_iter().find(|p| p.0 == id).map(|p| (p.1, p.2)).unwrap_or((100, 100)),
        };
        let class = wide(WIDGET_CLASS);
        let title = wide(id.title());
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                PCWSTR(class.as_ptr()),
                PCWSTR(title.as_ptr()),
                WS_POPUP,
                x,
                y,
                10,
                10,
                None,
                None,
                Some(hinstance()),
                None,
            )
        };
        let Ok(hwnd) = hwnd else { return };
        platform::pin_to_desktop(hwnd);
        let scale = dpi_scale(hwnd);
        let h = self.want_height(id);
        let rect = PRect { x, y, w: (id.size().0 as f32 * scale).round() as i32, h: (h * scale).round() as i32 };
        self.widgets.push(Widget {
            id,
            hwnd,
            surf: Surface::new(),
            zones: Vec::new(),
            hover: None,
            pressed: None,
            drag: None,
            tracking: false,
            h,
            rect,
        });
        let i = self.widgets.len() - 1;
        self.paint(i);
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(hwnd, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
        if id == Id::System {
            system::set_active(true);
        }
    }

    fn set_visible(&mut self, id: Id, visible: bool) {
        self.settings.widget(id.name()).visible = visible;
        settings::save(&self.settings);
        if visible {
            self.create_widget(id);
        } else if let Some(i) = self.idx_of(id) {
            let w = self.widgets.remove(i);
            if id == Id::System {
                self.expansion = None;
                EXPANDED.with(|e| e.set(0));
                system::set_active(false);
            }
            unsafe {
                let _ = DestroyWindow(w.hwnd);
            }
        }
    }

    fn reset_layout(&mut self) {
        if self.expansion.is_some() {
            self.collapse();
        }
        for (id, x, y) in self.default_positions() {
            if let Some(i) = self.idx_of(id) {
                self.place(i, x, y);
            }
        }
        settings::save(&self.settings);
    }

    // ---------- system widget hover expansion ----------

    /// Grow the system widget to its full height on top of its neighbours without moving them.
    fn expand(&mut self) {
        let Some(i) = self.idx_of(Id::System) else { return };
        let hwnd = self.widgets[i].hwnd;
        let first = self.expansion.is_none();
        let orig = self.expansion.as_ref().map_or(self.widgets[i].rect, |e| e.orig);
        if first {
            // Mark as expanded first so the full layout is measured.
            self.expansion = Some(Expansion { orig, at: (orig.x, orig.y) });
            EXPANDED.with(|e| e.set(hwnd.0 as isize));
            let _ = self.sys_wake.send(()); // fresh numbers for the full view
            unsafe {
                let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
        let scale = dpi_scale(hwnd);
        let h = self.want_height(Id::System);
        let ph = (h * scale).round() as i32;
        let (wa, _) = work_area_of(orig);
        let cur = self.widgets[i].rect;
        let mut y = layout::expanded_y(orig, ph, wa, scale as f64);
        let mut x = orig.x;
        if let Some(e) = &self.expansion {
            if !first && (cur.x, cur.y) != e.at {
                // dragged while open: stay where the user put it
                (x, y) = (cur.x, cur.y);
            }
        }
        self.expansion = Some(Expansion { orig, at: (x, y) });
        let w = &mut self.widgets[i];
        w.h = h;
        w.rect = PRect { x, y, w: orig.w, h: ph };
        self.paint(i);
    }

    /// Back to the compact size at the original spot (or where the user dragged it).
    fn collapse(&mut self) {
        let Some(e) = self.expansion.take() else { return };
        EXPANDED.with(|c| c.set(0));
        let Some(i) = self.idx_of(Id::System) else { return };
        let hwnd = self.widgets[i].hwnd;
        let cur = self.widgets[i].rect;
        let (x, y) = if (cur.x, cur.y) != e.at { (cur.x, cur.y) } else { (e.orig.x, e.orig.y) };
        let scale = dpi_scale(hwnd);
        let h = Id::System.size().1 as f32;
        let w = &mut self.widgets[i];
        w.h = h;
        w.rect = PRect { x, y, w: e.orig.w, h: (h * scale).round() as i32 };
        unsafe {
            let _ = SetWindowPos(hwnd, Some(HWND_NOTOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            let _ = SetWindowPos(hwnd, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
        self.paint(i);
    }

    // ---------- data ----------

    fn fetch_weather(&self) {
        for c in self.settings.cities.clone() {
            let ctrl = self.ctrl.0 as isize;
            std::thread::spawn(move || {
                // one retry after a short pause (flaky network, sleep/resume)
                let f = weather::fetch(&c).or_else(|| {
                    std::thread::sleep(std::time::Duration::from_secs(20));
                    weather::fetch(&c)
                });
                if let Some(f) = f {
                    post(HWND(ctrl as _), WM_WEATHER, (c.key(), f));
                }
            });
        }
    }

    fn load_wallpaper(&self) {
        let mons: Vec<(RECT, f32)> = platform::monitors().iter().map(|m| (m.rect, m.scale)).collect();
        let ctrl = self.ctrl.0 as isize;
        std::thread::spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            let walls = wallpaper::build(&mons);
            post(HWND(ctrl as _), WM_WALL, walls);
        });
    }

    fn fetch_rates(&self) {
        let ctrl = self.ctrl.0 as isize;
        std::thread::spawn(move || post(HWND(ctrl as _), WM_RATES, rates::fetch()));
    }

    fn refresh_all(&mut self) {
        self.fetch_rates();
        self.load_wallpaper();
        self.fetch_weather();
    }

    /// The list of cities changed (editor or click in the weather widget).
    pub fn cities_changed(&mut self) {
        settings::save(&self.settings);
        let missing: Vec<City> =
            self.settings.cities.iter().filter(|c| !self.weather.contains_key(&c.key())).cloned().collect();
        for c in missing {
            let ctrl = self.ctrl.0 as isize;
            std::thread::spawn(move || {
                if let Some(f) = weather::fetch(&c) {
                    post(HWND(ctrl as _), WM_WEATHER, (c.key(), f));
                }
            });
        }
        self.update(Id::Weather);
        if let Some(e) = &self.cities {
            e.invalidate();
        }
    }

    fn make_primary(&mut self, i: usize) {
        if i < self.settings.cities.len() {
            let c = self.settings.cities.remove(i);
            self.settings.cities.insert(0, c);
            self.cities_changed();
        }
    }

    fn quit(&mut self) {
        settings::save(&self.settings);
        remove_tray(self.ctrl);
        unsafe { PostQuitMessage(0) };
    }
}

// ---------- tray & menus ----------

const CMD_TOGGLE: usize = 1; // + index in Id::ALL
const CMD_CITIES: usize = 10;
const CMD_REFRESH: usize = 11;
const CMD_RESET: usize = 12;
const CMD_AUTOSTART: usize = 13;
const CMD_QUIT: usize = 14;
const CMD_HIDE: usize = 20; // + index in Id::ALL

fn app_icon(big: bool) -> Option<HICON> {
    unsafe {
        if big {
            LoadIconMetric(Some(hinstance()), PCWSTR(1 as _), windows::Win32::UI::Controls::LIM_LARGE).ok()
        } else {
            LoadIconMetric(Some(hinstance()), PCWSTR(1 as _), LIM_SMALL).ok()
        }
    }
}

fn tray_data(ctrl: HWND) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: ctrl,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: app_icon(false).unwrap_or_default(),
        ..Default::default()
    };
    for (i, c) in "Win11Widget".encode_utf16().enumerate() {
        nid.szTip[i] = c;
    }
    nid
}

fn add_tray(ctrl: HWND) {
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &tray_data(ctrl));
    }
}

fn remove_tray(ctrl: HWND) {
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &tray_data(ctrl));
    }
}

fn menu_item(m: HMENU, id: usize, text: &str, checked: bool) {
    let t = wide(text);
    let flags = MF_STRING | if checked { MF_CHECKED } else { MF_UNCHECKED };
    unsafe {
        let _ = AppendMenuW(m, flags, id, PCWSTR(t.as_ptr()));
    }
}
fn menu_sep(m: HMENU) {
    unsafe {
        let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
    }
}

/// Show a popup menu at the cursor; returns the chosen command (0 = none).
/// Must be called without the app state borrowed (it runs a modal loop).
fn popup(ctrl: HWND, build: impl FnOnce(HMENU)) -> usize {
    unsafe {
        let Ok(m) = CreatePopupMenu() else { return 0 };
        build(m);
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(ctrl);
        let cmd =
            TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, None, ctrl, None).0 as usize;
        let _ = PostMessageW(Some(ctrl), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(m);
        cmd
    }
}

fn tray_menu(ctrl: HWND) {
    let Some(vis) = with(|a| Id::ALL.map(|id| a.idx_of(id).is_some())) else { return };
    let auto = platform::autostart_enabled();
    let cmd = popup(ctrl, |m| {
        for (i, id) in Id::ALL.iter().enumerate() {
            menu_item(m, CMD_TOGGLE + i, id.title(), vis[i]);
        }
        menu_sep(m);
        menu_item(m, CMD_CITIES, "Города…", false);
        menu_item(m, CMD_REFRESH, "Обновить погоду", false);
        menu_item(m, CMD_RESET, "Расставить заново", false);
        menu_sep(m);
        menu_item(m, CMD_AUTOSTART, "Запускать вместе с Windows", auto);
        menu_sep(m);
        menu_item(m, CMD_QUIT, "Выход", false);
    });
    run_command(cmd);
}

fn widget_menu(ctrl: HWND, id: Id) {
    let idx = Id::ALL.iter().position(|x| *x == id).unwrap_or(0);
    let cmd = popup(ctrl, |m| {
        menu_item(m, CMD_HIDE + idx, "Скрыть виджет", false);
        menu_item(m, CMD_CITIES, "Города…", false);
        menu_item(m, CMD_REFRESH, "Обновить", false);
        menu_item(m, CMD_RESET, "Расставить заново", false);
        menu_sep(m);
        menu_item(m, CMD_QUIT, "Выход", false);
    });
    run_command(cmd);
}

fn run_command(cmd: usize) {
    match cmd {
        0 => {}
        CMD_CITIES => cities::open(),
        CMD_REFRESH => {
            with(|a| a.refresh_all());
        }
        CMD_RESET => {
            with(|a| a.reset_layout());
        }
        CMD_AUTOSTART => platform::set_autostart(!platform::autostart_enabled()),
        CMD_QUIT => {
            with(|a| a.quit());
        }
        c if (CMD_TOGGLE..CMD_TOGGLE + Id::ALL.len()).contains(&c) => {
            let id = Id::ALL[c - CMD_TOGGLE];
            with(|a| {
                let vis = a.idx_of(id).is_some();
                a.set_visible(id, !vis);
            });
        }
        c if (CMD_HIDE..CMD_HIDE + Id::ALL.len()).contains(&c) => {
            let id = Id::ALL[c - CMD_HIDE];
            with(|a| a.set_visible(id, false));
        }
        _ => {}
    }
}

// ---------- window procedures ----------

fn lparam_point(lp: LPARAM) -> (i32, i32) {
    ((lp.0 & 0xffff) as i16 as i32, ((lp.0 >> 16) & 0xffff) as i16 as i32)
}

fn arm_minute_timer(ctrl: HWND) {
    let t = unsafe { GetLocalTime() };
    let ms = 60_000 - (t.wSecond as u32 * 1000 + t.wMilliseconds as u32) + 15;
    unsafe {
        SetTimer(Some(ctrl), T_MINUTE, ms, None);
    }
}

extern "system" fn ctrl_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TRAY => {
                let ev = (lp.0 & 0xffff) as u32;
                if ev == WM_LBUTTONUP || ev == WM_RBUTTONUP {
                    tray_menu(hwnd);
                }
                LRESULT(0)
            }
            WM_TIMER => {
                match wp.0 {
                    T_MINUTE => {
                        arm_minute_timer(hwnd);
                        with(|a| {
                            a.paint_id(Id::Weather); // local times of other cities
                        });
                    }
                    T_PROGRESS => {
                        with(|a| {
                            if a.media.active && a.media.playing {
                                a.paint_id(Id::Media);
                            }
                        });
                    }
                    T_WEATHER => {
                        with(|a| a.fetch_weather());
                    }
                    T_RATES => {
                        with(|a| a.fetch_rates());
                    }
                    T_SAVE => {
                        let _ = KillTimer(Some(hwnd), T_SAVE);
                        with(|a| settings::save(&a.settings));
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_MEDIA => {
                with(|a| {
                    let (st, art, gen) = {
                        let g = media::SHARED.lock().unwrap();
                        (g.state.clone(), (g.art_gen != a.art_gen).then(|| g.art.clone()), g.art_gen)
                    };
                    a.media = st;
                    if let Some(img) = art {
                        a.art = img.as_ref().and_then(|i| a.gfx.bitmap(i));
                        a.art_gen = gen;
                    }
                    a.paint_id(Id::Media);
                });
                LRESULT(0)
            }
            WM_STATS => {
                let s = take::<system::Stats>(lp);
                with(|a| {
                    a.stats = Some(*s);
                    #[cfg(debug_assertions)]
                    if std::env::var_os("W11W_EXPAND").is_some() && a.expansion.is_none() {
                        a.expand();
                    }
                    a.update(Id::System);
                });
                LRESULT(0)
            }
            WM_WEATHER => {
                let b = take::<(String, weather::Forecast)>(lp);
                with(|a| {
                    let (k, f) = *b;
                    a.weather.insert(k, f);
                    a.update(Id::Weather);
                });
                LRESULT(0)
            }
            WM_RATES => {
                let r = take::<rates::Rates>(lp);
                with(|a| {
                    // keep the last good value of a source that failed this time
                    let r = *r;
                    a.rates.usd = r.usd.or(a.rates.usd);
                    a.rates.btc = r.btc.or(a.rates.btc.take());
                    a.rates.gram = r.gram.or(a.rates.gram.take());
                    a.paint_id(Id::Rates);
                });
                LRESULT(0)
            }
            WM_WALL => {
                let walls = take::<Vec<wallpaper::Wall>>(lp);
                with(|a| {
                    a.walls = *walls;
                    a.glass.clear();
                    for i in 0..a.widgets.len() {
                        a.paint(i);
                    }
                });
                LRESULT(0)
            }
            WM_OPEN_CITIES => {
                cities::open();
                LRESULT(0)
            }
            WM_SETTINGCHANGE => {
                // SPI_SETDESKWALLPAPER
                if wp.0 == 20 {
                    with(|a| a.load_wallpaper());
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_DISPLAYCHANGE => {
                with(|a| {
                    a.load_wallpaper();
                    for i in 0..a.widgets.len() {
                        a.paint(i);
                    }
                });
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            _ => {
                let tc = with(|a| a.taskbar_created).unwrap_or(0);
                if tc != 0 && msg == tc {
                    // Explorer restarted: tray icon and the desktop window are new.
                    add_tray(hwnd);
                    with(|a| {
                        for w in &a.widgets {
                            platform::pin_to_desktop(w.hwnd);
                        }
                    });
                    return LRESULT(0);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
        }
    }
}

enum After {
    None,
    Menu(Id),
    Release,
    Media(media::Cmd),
    City(usize),
    EditCities,
}

extern "system" fn widget_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_WINDOWPOSCHANGING => {
                // Stay at the bottom of the z-order (just above the desktop), except while
                // the system widget is expanded over its neighbours.
                if EXPANDED.with(|e| e.get()) != hwnd.0 as isize {
                    let p = lp.0 as *mut WINDOWPOS;
                    (*p).hwndInsertAfter = HWND_BOTTOM;
                    (*p).flags &= !SWP_NOZORDER;
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_DPICHANGED => {
                let _ = PostMessageW(Some(hwnd), WM_RERENDER, WPARAM(0), LPARAM(0));
                LRESULT(0)
            }
            WM_RERENDER => {
                with(|a| {
                    if let Some(i) = a.idx(hwnd) {
                        let s = dpi_scale(hwnd);
                        let id = a.widgets[i].id;
                        let w = &mut a.widgets[i];
                        w.rect.w = (id.size().0 as f32 * s).round() as i32;
                        w.rect.h = (w.h * s).round() as i32;
                        a.paint(i);
                    }
                });
                LRESULT(0)
            }
            WM_SETCURSOR => {
                let hand =
                    with(|a| a.idx(hwnd).map(|i| a.widgets[i].hover.is_some()).unwrap_or(false)).unwrap_or(false);
                let cur = LoadCursorW(None, if hand { IDC_HAND } else { IDC_ARROW });
                if let Ok(c) = cur {
                    SetCursor(Some(c));
                }
                LRESULT(1)
            }
            WM_MOUSEMOVE => {
                let (mx, my) = lparam_point(lp);
                with(|a| {
                    let Some(i) = a.idx(hwnd) else { return };
                    let scale = dpi_scale(hwnd);
                    if !a.widgets[i].tracking {
                        let mut t = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        let _ = TrackMouseEvent(&mut t);
                        a.widgets[i].tracking = true;
                        if a.widgets[i].id == Id::System {
                            let _ = KillTimer(Some(hwnd), T_CLOSE);
                            if a.expansion.is_none() {
                                SetTimer(Some(hwnd), T_OPEN, 250, None);
                            }
                        }
                    }
                    let r = a.widgets[i].rect;
                    if let Some(d) = &mut a.widgets[i].drag {
                        let pt = POINT { x: r.x + mx, y: r.y + my };
                        let (x, y) = (d.win.0 + pt.x - d.cursor.x, d.win.1 + pt.y - d.cursor.y);
                        if d.moved || (pt.x - d.cursor.x).abs() + (pt.y - d.cursor.y).abs() > 0 {
                            d.moved = true;
                            a.place(i, x, y);
                        }
                        return;
                    }
                    let (lx, ly) = (mx as f32 / scale, my as f32 / scale);
                    let hover = a.widgets[i].zones.iter().find(|(r, _)| r.contains(lx, ly)).map(|(_, h)| *h);
                    if hover != a.widgets[i].hover {
                        a.widgets[i].hover = hover;
                        a.paint(i);
                    }
                });
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                with(|a| {
                    let Some(i) = a.idx(hwnd) else { return };
                    a.widgets[i].tracking = false;
                    if a.widgets[i].id == Id::System {
                        let _ = KillTimer(Some(hwnd), T_OPEN);
                        if a.expansion.is_some() && a.widgets[i].drag.is_none() {
                            SetTimer(Some(hwnd), T_CLOSE, 300, None);
                        }
                    }
                    if a.widgets[i].hover.is_some() && a.widgets[i].drag.is_none() {
                        a.widgets[i].hover = None;
                        a.paint(i);
                    }
                });
                LRESULT(0)
            }
            WM_TIMER => {
                let _ = KillTimer(Some(hwnd), wp.0);
                match wp.0 {
                    T_OPEN => {
                        with(|a| a.expand());
                    }
                    T_CLOSE => {
                        with(|a| a.collapse());
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let (mx, my) = lparam_point(lp);
                with(|a| {
                    let Some(i) = a.idx(hwnd) else { return };
                    let scale = dpi_scale(hwnd);
                    let (lx, ly) = (mx as f32 / scale, my as f32 / scale);
                    let hit = a.widgets[i].zones.iter().find(|(r, _)| r.contains(lx, ly)).map(|(_, h)| *h);
                    let w = &mut a.widgets[i];
                    if let Some(h) = hit {
                        w.pressed = Some(h);
                        a.paint(i);
                    } else {
                        let pt = POINT { x: w.rect.x + mx, y: w.rect.y + my };
                        w.drag = Some(Drag { cursor: pt, win: (w.rect.x, w.rect.y), moved: false });
                    }
                });
                SetCapture(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (mx, my) = lparam_point(lp);
                let after = with(|a| {
                    let Some(i) = a.idx(hwnd) else { return After::Release };
                    let scale = dpi_scale(hwnd);
                    let (lx, ly) = (mx as f32 / scale, my as f32 / scale);
                    if let Some(d) = a.widgets[i].drag.take() {
                        if d.moved {
                            // snap to neighbours / screen edges
                            let me = a.widgets[i].rect;
                            let others: Vec<PRect> =
                                a.widgets.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, w)| w.rect).collect();
                            let (wa, sc) = work_area_of(me);
                            let (x, y) = layout::snap(me, &others, wa, sc as f64);
                            a.place(i, x, y);
                            if a.widgets[i].id == Id::System && a.expansion.is_some() {
                                // moved while open: the compact widget goes there too
                                if let Some(e) = &mut a.expansion {
                                    e.orig.x = x;
                                    e.orig.y = y;
                                    e.at = (x, y);
                                }
                            }
                            settings::save(&a.settings);
                        }
                        return After::Release;
                    }
                    let pressed = a.widgets[i].pressed.take();
                    let hit = a.widgets[i].zones.iter().find(|(r, _)| r.contains(lx, ly)).map(|(_, h)| *h);
                    a.paint(i);
                    match pressed {
                        Some(p) if Some(p) == hit => match p {
                            Hit::Prev => After::Media(media::Cmd::Prev),
                            Hit::Next => After::Media(media::Cmd::Next),
                            Hit::Toggle => {
                                a.media.playing = !a.media.playing; // instant feedback; the next poll confirms
                                a.paint(i);
                                After::Media(media::Cmd::Toggle)
                            }
                            Hit::City(c) => After::City(c),
                            Hit::EditCities => After::EditCities,
                        },
                        _ => After::Release,
                    }
                })
                .unwrap_or(After::Release);
                let _ = ReleaseCapture();
                match after {
                    After::Media(c) => {
                        with(|a| {
                            let _ = a.media_tx.send(c);
                        });
                    }
                    After::City(c) => {
                        with(|a| a.make_primary(c));
                    }
                    After::EditCities => cities::open(),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_CAPTURECHANGED => {
                with(|a| {
                    if let Some(i) = a.idx(hwnd) {
                        a.widgets[i].drag = None;
                        if a.widgets[i].pressed.take().is_some() {
                            a.paint(i);
                        }
                    }
                });
                LRESULT(0)
            }
            WM_RBUTTONUP => {
                let after =
                    with(|a| a.idx(hwnd).map(|i| After::Menu(a.widgets[i].id))).flatten().unwrap_or(After::None);
                if let After::Menu(id) = after {
                    let ctrl = with(|a| a.ctrl).unwrap_or_default();
                    widget_menu(ctrl, id);
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

fn register(class: &str, proc_: WNDPROC, bg: Option<HBRUSH>) {
    let name = wide(class);
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: proc_,
        hInstance: hinstance(),
        lpszClassName: PCWSTR(name.as_ptr()),
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_default() },
        hIcon: app_icon(true).unwrap_or_default(),
        hIconSm: app_icon(false).unwrap_or_default(),
        hbrBackground: bg.unwrap_or_default(),
        ..Default::default()
    };
    unsafe {
        RegisterClassExW(&wc);
    }
}

fn main() {
    unsafe {
        // Single instance: a second launch just opens the cities editor of the running one.
        let mutex_name = wide("Local\\Win11Widget.Native.Instance");
        let _mutex = CreateMutexW(None, true, PCWSTR(mutex_name.as_ptr()));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let class = wide(CTRL_CLASS);
            if let Ok(h) = FindWindowW(PCWSTR(class.as_ptr()), PCWSTR::null()) {
                let _ = PostMessageW(Some(h), WM_OPEN_CITIES, WPARAM(0), LPARAM(0));
            }
            return;
        }
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        platform::dark_menus();

        register(CTRL_CLASS, Some(ctrl_proc), None);
        register(WIDGET_CLASS, Some(widget_proc), None);
        cities::register();

        let class = wide(CTRL_CLASS);
        let Ok(ctrl) = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            PCWSTR(class.as_ptr()),
            PCWSTR(wide("Win11Widget").as_ptr()),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance()),
            None,
        ) else {
            return;
        };

        let Ok(gfx) = Gfx::new() else { return };
        let mut settings = settings::load();
        for id in Id::ALL {
            settings.widget(id.name());
        }
        let ctrl_i = ctrl.0 as isize;
        let media_tx = media::spawn(move || {
            let _ = PostMessageW(Some(HWND(ctrl_i as _)), WM_MEDIA, WPARAM(0), LPARAM(0));
        });
        let sys_wake = system::spawn(move |s| post(HWND(ctrl_i as _), WM_STATS, s));

        let app = App {
            gfx,
            settings,
            widgets: Vec::new(),
            ctrl,
            weather: HashMap::new(),
            rates: Default::default(),
            stats: None,
            media: Default::default(),
            art: None,
            art_gen: 0,
            media_tx,
            sys_wake,
            walls: Vec::new(),
            glass: Vec::new(),
            expansion: None,
            cities: None,
            taskbar_created: RegisterWindowMessageW(PCWSTR(wide("TaskbarCreated").as_ptr())),
        };
        APP.with(|a| *a.borrow_mut() = Some(app));

        with(|a| {
            let visible: Vec<Id> = Id::ALL.iter().copied().filter(|id| a.settings.visible(id.name())).collect();
            for id in visible {
                a.create_widget(id);
            }
            a.load_wallpaper();
            a.fetch_weather();
            a.fetch_rates();
        });
        add_tray(ctrl);
        arm_minute_timer(ctrl);
        SetTimer(Some(ctrl), T_PROGRESS, 1000, None);
        SetTimer(Some(ctrl), T_WEATHER, WEATHER_EVERY_MS, None);
        SetTimer(Some(ctrl), T_RATES, RATES_EVERY_MS, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if let Some(ed) = with(|a| a.cities.as_ref().map(|e| e.hwnd)).flatten() {
                if IsDialogMessageW(ed, &msg).as_bool() {
                    continue;
                }
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        APP.with(|a| a.borrow_mut().take());
    }
}
