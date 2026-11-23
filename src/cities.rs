//! "Города" window: search (any language), add, make main (★), reorder, delete.
//! A normal window drawn with Direct2D, plus one native EDIT control for typing.

use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::*,
        Graphics::{
            Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE},
            Gdi::*,
        },
        UI::{
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::{SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT},
            WindowsAndMessaging::*,
        },
    },
};

use crate::{
    gfx::{Align, Color, Family, Gfx, Rect, Surface},
    post,
    settings::City,
    weather, wide, with, WM_SEARCH,
};

const CLASS: &str = "Win11Widget.Native.Cities";
const BG: Color = Color::hex(0x1f1f23);
const PANEL: Color = Color::hex(0x2a2a30);
const LINE: Color = Color::hex(0x3a3a42);
const FG2: Color = Color::hex(0xa1a1aa);
const ACCENT: Color = Color::hex(0x0a84ff);
const GOLD: Color = Color::hex(0xffd60a);
const T_SEARCH: usize = 1;
const EDIT_ID: usize = 100;
const EN_CHANGE: u32 = 0x0300;
const EN_SETFOCUS: u32 = 0x0100;
const EN_KILLFOCUS: u32 = 0x0200;
const EM_SETCUEBANNER: u32 = 0x1501;
const WM_MOUSELEAVE: u32 = 0x02A3;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Act {
    Main(usize),
    Up(usize),
    Down(usize),
    Del(usize),
    Add(usize),
}

enum Results {
    Hidden,
    Found(Vec<City>),
    NotFound,
    Offline,
}

pub struct Editor {
    pub hwnd: HWND,
    edit: HWND,
    font: HFONT,
    edit_brush: HBRUSH,
    surf: Surface,
    results: Results,
    seq: u32,
    focused: bool,
    hover: Option<Act>,
    zones: Vec<(Rect, Act)>,
    scroll_mine: f32,
    scroll_res: f32,
    /// list areas, for wheel scrolling
    mine_area: Rect,
    res_area: Rect,
    tracking: bool,
}

impl Editor {
    pub fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.font.into());
            let _ = DeleteObject(self.edit_brush.into());
        }
    }
}

pub fn register() {
    let name = wide(CLASS);
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(proc_),
        hInstance: crate::hinstance(),
        lpszClassName: PCWSTR(name.as_ptr()),
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_default() },
        hIcon: crate::app_icon(true).unwrap_or_default(),
        hIconSm: crate::app_icon(false).unwrap_or_default(),
        ..Default::default()
    };
    unsafe {
        RegisterClassExW(&wc);
    }
}

fn scale_of(hwnd: HWND) -> f32 {
    let d = unsafe { GetDpiForWindow(hwnd) };
    if d == 0 {
        1.0
    } else {
        d as f32 / 96.0
    }
}

fn make_font(scale: f32) -> HFONT {
    let face = wide("Segoe UI Variable Text");
    unsafe {
        CreateFontW(
            -(14.0 * scale).round() as i32,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            PCWSTR(face.as_ptr()),
        )
    }
}

fn colorref(c: Color) -> COLORREF {
    COLORREF((c.0 * 255.0) as u32 | ((c.1 * 255.0) as u32) << 8 | ((c.2 * 255.0) as u32) << 16)
}

/// Open (or bring to front) the editor.
pub fn open() {
    if let Some(Some(h)) = with(|a| a.cities.as_ref().map(|e| e.hwnd)) {
        unsafe {
            let _ = ShowWindow(h, SW_RESTORE);
            let _ = SetForegroundWindow(h);
        }
        return;
    }
    let prim = crate::platform::primary();
    let s = prim.scale;
    let (cw, ch) = ((460.0 * s) as i32, (600.0 * s) as i32);
    let style = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN;
    let mut r = RECT { left: 0, top: 0, right: cw, bottom: ch };
    unsafe {
        let _ = AdjustWindowRectEx(&mut r, style, false, WINDOW_EX_STYLE(0));
    }
    let (ww, wh) = (r.right - r.left, r.bottom - r.top);
    let wa = prim.work;
    let x = wa.left + ((wa.right - wa.left) - ww) / 2;
    let y = wa.top + ((wa.bottom - wa.top) - wh) / 2;
    let class = wide(CLASS);
    let title = wide("Города — Win11Widget");
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
            style,
            x,
            y,
            ww,
            wh,
            None,
            None,
            Some(crate::hinstance()),
            None,
        )
    };
    let Ok(hwnd) = hwnd else { return };
    unsafe {
        let dark: i32 = 1;
        let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(20), &dark as *const i32 as _, 4);
        let edit_class = wide("EDIT");
        let edit = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(edit_class.as_ptr()),
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            0,
            0,
            10,
            10,
            Some(hwnd),
            Some(HMENU(EDIT_ID as _)),
            Some(crate::hinstance()),
            None,
        )
        .unwrap_or_default();
        let scale = scale_of(hwnd);
        let font = make_font(scale);
        SendMessageW(edit, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
        let cue = wide("Найти город, например «Берлин» или «Paris»");
        SendMessageW(edit, EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(cue.as_ptr() as isize)));
        let editor = Editor {
            hwnd,
            edit,
            font,
            edit_brush: CreateSolidBrush(colorref(PANEL)),
            surf: Surface::new(),
            results: Results::Hidden,
            seq: 0,
            focused: false,
            hover: None,
            zones: Vec::new(),
            scroll_mine: 0.0,
            scroll_res: 0.0,
            mine_area: Rect::default(),
            res_area: Rect::default(),
            tracking: false,
        };
        with(|a| a.cities = Some(editor));
        layout_edit(hwnd);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(edit));
    }
}

// ---------- geometry (logical px) ----------

struct Geo {
    w: f32,
    h: f32,
    input: Rect,
    results: Option<Rect>,
    h2_y: f32,
    mine: Rect,
    hint_y: f32,
}

const PAD_X: f32 = 22.0;
const PAD_Y: f32 = 20.0;

fn row_h(g: &Gfx) -> f32 {
    9.0 + (g.normal_lh(14.0) + g.normal_lh(12.0)).max(30.0) + 9.0
}

fn hint_text() -> &'static str {
    "Основной город (★) показывается крупно в виджете «Погода». Сделать основным — ☆ здесь или клик по городу в самом виджете."
}

fn geometry(g: &Gfx, w: f32, h: f32, results: &Results) -> Geo {
    let cw = w - 2.0 * PAD_X;
    let mut y = PAD_Y + g.normal_lh(20.0) + 14.0;
    let input = Rect::new(PAD_X, y, cw, 20.0 + g.normal_lh(14.0) + 2.0);
    y += input.h;
    let rows = match results {
        Results::Hidden => 0,
        Results::Found(v) => v.len().max(1),
        _ => 1,
    };
    let results = if rows > 0 {
        let rh = (row_h(g) + 1.0) * rows as f32 - 1.0;
        let r = Rect::new(PAD_X, y + 8.0, cw, rh.min(230.0));
        y = r.y + r.h;
        Some(r)
    } else {
        None
    };
    y += 14.0;
    let h2_y = y;
    y += g.normal_lh(12.0) + 6.0;
    let hint_h = g.text(hint_text(), 12.0).family(Family::Text).width(cw).wrap().measure_height();
    let hint_y = h - PAD_Y - hint_h;
    let mine = Rect::new(PAD_X, y, cw, (hint_y - 14.0 - y).max(40.0));
    Geo { w, h, input, results, h2_y, mine, hint_y }
}

fn client_logical(hwnd: HWND) -> (f32, f32, f32) {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    let s = scale_of(hwnd);
    (rc.right as f32 / s, rc.bottom as f32 / s, s)
}

fn layout_edit(hwnd: HWND) {
    let pos = with(|a| {
        let e = a.cities.as_ref()?;
        let (w, h, s) = client_logical(hwnd);
        let geo = geometry(&a.gfx, w, h, &e.results);
        let lh = (14.0 * 1.36 * s).round() as i32;
        let x = ((geo.input.x + 13.0) * s).round() as i32;
        let y = ((geo.input.y + geo.input.h / 2.0) * s).round() as i32 - lh / 2;
        let ew = ((geo.input.w - 26.0) * s).round() as i32;
        Some((e.edit, x, y, ew, lh))
    })
    .flatten();
    if let Some((edit, x, y, w, h)) = pos {
        unsafe {
            let _ = MoveWindow(edit, x, y, w, h, true);
        }
    }
}

// ---------- drawing ----------

fn button(g: &Gfx, r: Rect, label: &str, bg: Color, fg: Color, size: f32, enabled: bool) {
    let a = if enabled { 1.0 } else { 0.35 };
    if bg.3 > 0.0 {
        g.fill_rrect(r, 6.0, bg.alpha(a));
    }
    let lh = g.normal_lh(size);
    g.text(label, size)
        .family(Family::Text)
        .color(fg.alpha(a))
        .width(r.w)
        .align(Align::Center)
        .draw(r.x, r.y + (r.h - lh) / 2.0);
}

fn paint(hwnd: HWND) {
    with(|a| {
        let crate::App { gfx, settings, cities, .. } = a;
        let Some(e) = cities.as_mut() else { return };
        let (w, h, s) = client_logical(hwnd);
        let geo = geometry(gfx, w, h, &e.results);
        let mut zones = Vec::new();
        let (pw, ph) = ((w * s).round() as i32, (h * s).round() as i32);
        let hover = e.hover;
        let list = &settings.cities;
        let mut mine_area = geo.mine;
        let mut res_area = Rect::default();
        gfx.draw(&mut e.surf, pw, ph, s, |g| {
            g.fill_rect(Rect::new(0.0, 0.0, geo.w, geo.h), BG);
            g.text("Города", 20.0).weight(600).draw(PAD_X, PAD_Y);

            // search box (the EDIT control sits inside)
            g.fill_rrect(geo.input, 8.0, PANEL);
            g.stroke_rrect(geo.input.inset(0.5), 7.5, 1.0, if e.focused { ACCENT } else { LINE });

            let rh = row_h(g);
            if let Some(rr) = geo.results {
                res_area = rr;
                g.fill_rrect(rr, 10.0, PANEL);
                g.push_clip_rrect(rr, 10.0);
                let msg = match &e.results {
                    Results::NotFound => Some("Ничего не найдено"),
                    Results::Offline => Some("Нет соединения с сервисом поиска"),
                    Results::Found(v) if v.is_empty() => Some("Ничего не найдено"),
                    _ => None,
                };
                if let Some(m) = msg {
                    let lh = g.normal_lh(12.0);
                    g.text(m, 12.0).family(Family::Text).color(FG2).draw(rr.x + 14.0, rr.y + (rh - lh) / 2.0);
                } else if let Results::Found(v) = &e.results {
                    let total = (rh + 1.0) * v.len() as f32 - 1.0;
                    e.scroll_res = e.scroll_res.clamp(0.0, (total - rr.h).max(0.0));
                    for (i, c) in v.iter().enumerate() {
                        let y = rr.y + i as f32 * (rh + 1.0) - e.scroll_res;
                        if i + 1 < v.len() {
                            g.fill_rect(Rect::new(rr.x, y + rh, rr.w, 1.0), LINE);
                        }
                        let tw = g.text("Добавить", 13.0).family(Family::Text).measure() + 20.0;
                        let br = Rect::new(rr.x + rr.w - 10.0 - tw, y + (rh - 30.0) / 2.0, tw, 30.0);
                        let hov = hover == Some(Act::Add(i));
                        button(
                            g,
                            br,
                            "Добавить",
                            if hov { Color::hex(0x3398ff) } else { ACCENT },
                            Color::white(1.0),
                            13.0,
                            true,
                        );
                        if br.y + br.h > rr.y && br.y < rr.y + rr.h {
                            zones.push((br, Act::Add(i)));
                        }
                        name_block(g, c, None, rr.x + 14.0, y, br.x - 8.0 - (rr.x + 14.0), rh);
                    }
                }
                g.pop_layer();
            }

            g.text("МОИ ГОРОДА", 12.0).family(Family::Text).weight(600).ls(0.5).color(FG2).draw(PAD_X, geo.h2_y);

            let mr = geo.mine;
            mine_area = mr;
            let total = if list.is_empty() { rh } else { (rh + 1.0) * list.len() as f32 - 1.0 };
            g.fill_rrect(mr, 10.0, PANEL);
            g.push_clip_rrect(mr, 10.0);
            e.scroll_mine = e.scroll_mine.clamp(0.0, (total - mr.h).max(0.0));
            if list.is_empty() {
                let lh = g.normal_lh(12.0);
                g.text("Пока пусто — найдите город выше", 12.0)
                    .family(Family::Text)
                    .color(FG2)
                    .draw(mr.x + 14.0, mr.y + (rh - lh) / 2.0);
            }
            let n = list.len();
            for (i, c) in list.iter().enumerate() {
                let y = mr.y + i as f32 * (rh + 1.0) - e.scroll_mine;
                if i + 1 < n {
                    g.fill_rect(Rect::new(mr.x, y + rh, mr.w, 1.0), LINE);
                }
                // buttons right to left: ✕ ↓ ↑ ★
                let by = y + (rh - 30.0) / 2.0;
                let mut bx = mr.x + mr.w - 10.0 - 30.0;
                let visible = by + 30.0 > mr.y && by < mr.y + mr.h;
                let mut btn = |g: &Gfx, act: Act, label: &str, enabled: bool, star: bool| {
                    let r = Rect::new(bx, by, 30.0, 30.0);
                    let hov = hover == Some(act) && enabled;
                    if star {
                        if hov {
                            g.fill_rrect(r, 6.0, LINE);
                        }
                        button(g, r, label, Color(0.0, 0.0, 0.0, 0.0), GOLD, 16.0, true);
                    } else {
                        button(
                            g,
                            r,
                            label,
                            if hov { Color::hex(0x4a4a54) } else { LINE },
                            Color::white(1.0),
                            13.0,
                            enabled,
                        );
                    }
                    if enabled && visible {
                        zones.push((r, act));
                    }
                    bx -= 38.0;
                };
                btn(g, Act::Del(i), "✕", true, false);
                btn(g, Act::Down(i), "↓", i + 1 < n, false);
                btn(g, Act::Up(i), "↑", i > 0, false);
                if i == 0 {
                    btn(g, Act::Main(0), "★", false, true);
                } else {
                    btn(g, Act::Main(i), "☆", true, true);
                }
                let nx = mr.x + 14.0;
                name_block(g, c, Some(i == 0), nx, y, bx + 30.0 - 8.0 - nx, rh);
            }
            g.pop_layer();

            g.text(hint_text(), 12.0)
                .family(Family::Text)
                .color(FG2)
                .width(geo.w - 2.0 * PAD_X)
                .wrap()
                .draw(PAD_X, geo.hint_y);
        });
        #[cfg(debug_assertions)]
        if let Some(dir) = std::env::var_os("W11W_DUMP") {
            e.surf.dump(&std::path::Path::new(&dir).join("cities.bmp"));
        }
        e.zones = zones;
        e.mine_area = mine_area;
        e.res_area = res_area;
    });
}

fn name_block(g: &Gfx, c: &City, main: Option<bool>, x: f32, y: f32, w: f32, rh: f32) {
    let (l1, l2) = (g.normal_lh(14.0), g.normal_lh(12.0));
    let top = y + (rh - l1 - l2) / 2.0;
    let badge = if main == Some(true) {
        g.text("ОСНОВНОЙ", 11.0).family(Family::Text).weight(600).measure() + 6.0
    } else {
        0.0
    };
    let nw = g.text(&c.name, 14.0).family(Family::Text).weight(600).width((w - badge).max(0.0)).ellipsis().draw(x, top);
    if badge > 0.0 {
        let lh11 = g.normal_lh(11.0);
        let base = g.baseline(14.0) - g.baseline(11.0);
        let _ = lh11;
        g.text("ОСНОВНОЙ", 11.0)
            .family(Family::Text)
            .weight(600)
            .color(ACCENT)
            .draw(x + nw.min(w - badge) + 6.0, top + base);
    }
    g.text(&c.place(), 12.0).family(Family::Text).color(FG2).width(w.max(0.0)).ellipsis().draw(x, top + l1);
}

// ---------- actions ----------

fn act(hwnd: HWND, a: Act) {
    let edit = with(|app| {
        let list = &mut app.settings.cities;
        match a {
            Act::Main(i) if i < list.len() => {
                let c = list.remove(i);
                list.insert(0, c);
            }
            Act::Up(i) if i > 0 && i < list.len() => list.swap(i - 1, i),
            Act::Down(i) if i + 1 < list.len() => list.swap(i, i + 1),
            Act::Del(i) if i < list.len() => {
                list.remove(i);
            }
            Act::Add(i) => {
                let found = match &app.cities.as_ref().map(|e| &e.results) {
                    Some(Results::Found(v)) => v.get(i).cloned(),
                    _ => None,
                };
                if let Some(c) = found {
                    if !list.iter().any(|x| (x.lat - c.lat).abs() < 0.01 && (x.lon - c.lon).abs() < 0.01) {
                        list.push(c);
                    }
                }
                if let Some(e) = app.cities.as_mut() {
                    e.results = Results::Hidden;
                    e.seq += 1;
                    e.hover = None;
                }
            }
            _ => return None,
        }
        app.cities_changed();
        app.cities.as_ref().map(|e| (e.edit, matches!(a, Act::Add(_))))
    })
    .flatten();
    if let Some((edit, added)) = edit {
        if added {
            unsafe {
                let _ = SetWindowTextW(edit, PCWSTR(wide("").as_ptr()));
                let _ = SetFocus(Some(edit));
            }
            layout_edit(hwnd);
        }
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
}

fn start_search(hwnd: HWND) {
    let Some((edit, seq)) = with(|a| {
        a.cities.as_mut().map(|e| {
            e.seq += 1;
            (e.edit, e.seq)
        })
    })
    .flatten() else {
        return;
    };
    let mut buf = [0u16; 256];
    let n = unsafe { GetWindowTextW(edit, &mut buf) } as usize;
    let text = String::from_utf16_lossy(&buf[..n]).trim().to_string();
    if text.chars().count() < 2 {
        with(|a| {
            if let Some(e) = a.cities.as_mut() {
                e.results = Results::Hidden;
            }
        });
        layout_edit(hwnd);
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        return;
    }
    let h = hwnd.0 as isize;
    std::thread::spawn(move || {
        let r = weather::search(&text);
        post(HWND(h as _), WM_SEARCH, (seq, r));
    });
}

fn hit(hwnd: HWND, lp: LPARAM) -> Option<Act> {
    let (x, y) = ((lp.0 & 0xffff) as i16 as f32, ((lp.0 >> 16) & 0xffff) as i16 as f32);
    let s = scale_of(hwnd);
    with(|a| a.cities.as_ref().and_then(|e| e.zones.iter().find(|(r, _)| r.contains(x / s, y / s)).map(|(_, z)| *z)))
        .flatten()
}

extern "system" fn proc_(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                paint(hwnd);
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                with(|a| {
                    if let Some(e) = &a.cities {
                        e.surf.blit(dc);
                    }
                });
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_SIZE => {
                layout_edit(hwnd);
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_GETMINMAXINFO => {
                let s = scale_of(hwnd);
                let mi = lp.0 as *mut MINMAXINFO;
                let mut r = RECT { left: 0, top: 0, right: (380.0 * s) as i32, bottom: (440.0 * s) as i32 };
                let _ = AdjustWindowRectEx(&mut r, WS_OVERLAPPEDWINDOW, false, WINDOW_EX_STYLE(0));
                (*mi).ptMinTrackSize = POINT { x: r.right - r.left, y: r.bottom - r.top };
                LRESULT(0)
            }
            WM_DPICHANGED => {
                let r = &*(lp.0 as *const RECT);
                let s = (wp.0 & 0xffff) as f32 / 96.0;
                let edit = with(|a| {
                    a.cities.as_mut().map(|e| {
                        let _ = DeleteObject(e.font.into());
                        e.font = make_font(s);
                        (e.edit, e.font)
                    })
                })
                .flatten();
                if let Some((edit, font)) = edit {
                    SendMessageW(edit, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
                }
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                LRESULT(0)
            }
            WM_CTLCOLOREDIT => {
                let dc = HDC(wp.0 as _);
                SetTextColor(dc, COLORREF(0x00ffffff));
                SetBkColor(dc, colorref(PANEL));
                let b = with(|a| a.cities.as_ref().map(|e| e.edit_brush)).flatten().unwrap_or_default();
                LRESULT(b.0 as isize)
            }
            WM_COMMAND => {
                let code = ((wp.0 >> 16) & 0xffff) as u32;
                let id = wp.0 & 0xffff;
                if id == EDIT_ID {
                    match code {
                        EN_CHANGE => {
                            SetTimer(Some(hwnd), T_SEARCH, 300, None);
                        }
                        EN_SETFOCUS | EN_KILLFOCUS => {
                            with(|a| {
                                if let Some(e) = a.cities.as_mut() {
                                    e.focused = code == EN_SETFOCUS;
                                }
                            });
                            let _ = InvalidateRect(Some(hwnd), None, false);
                        }
                        _ => {}
                    }
                } else if id == 2 {
                    // IDCANCEL (Esc)
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_TIMER => {
                let _ = KillTimer(Some(hwnd), wp.0);
                if wp.0 == T_SEARCH {
                    start_search(hwnd);
                }
                LRESULT(0)
            }
            WM_SEARCH => {
                let b = crate::take::<(u32, Option<Vec<City>>)>(lp);
                let (seq, r) = *b;
                with(|a| {
                    if let Some(e) = a.cities.as_mut() {
                        if e.seq == seq {
                            e.results = match r {
                                None => Results::Offline,
                                Some(v) if v.is_empty() => Results::NotFound,
                                Some(v) => Results::Found(v),
                            };
                            e.scroll_res = 0.0;
                        }
                    }
                });
                layout_edit(hwnd);
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let h = hit(hwnd, lp);
                let changed = with(|a| {
                    a.cities.as_mut().map(|e| {
                        if !e.tracking {
                            let mut t = TRACKMOUSEEVENT {
                                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                                dwFlags: TME_LEAVE,
                                hwndTrack: hwnd,
                                dwHoverTime: 0,
                            };
                            let _ = TrackMouseEvent(&mut t);
                            e.tracking = true;
                        }
                        let c = e.hover != h;
                        e.hover = h;
                        c
                    })
                })
                .flatten()
                .unwrap_or(false);
                if changed {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                with(|a| {
                    if let Some(e) = a.cities.as_mut() {
                        e.tracking = false;
                        e.hover = None;
                    }
                });
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_SETCURSOR if (lp.0 & 0xffff) as u32 == HTCLIENT => {
                let hand = with(|a| a.cities.as_ref().map(|e| e.hover.is_some())).flatten().unwrap_or(false);
                if let Ok(c) = LoadCursorW(None, if hand { IDC_HAND } else { IDC_ARROW }) {
                    SetCursor(Some(c));
                }
                LRESULT(1)
            }
            WM_LBUTTONUP => {
                if let Some(a) = hit(hwnd, lp) {
                    act(hwnd, a);
                }
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = ((wp.0 >> 16) & 0xffff) as i16 as f32 / 120.0;
                let mut pt = POINT { x: (lp.0 & 0xffff) as i16 as i32, y: ((lp.0 >> 16) & 0xffff) as i16 as i32 };
                let _ = ScreenToClient(hwnd, &mut pt);
                let s = scale_of(hwnd);
                let (x, y) = (pt.x as f32 / s, pt.y as f32 / s);
                with(|a| {
                    if let Some(e) = a.cities.as_mut() {
                        if e.res_area.contains(x, y) {
                            e.scroll_res -= delta * 53.0;
                        } else if e.mine_area.contains(x, y) {
                            e.scroll_mine -= delta * 53.0;
                        }
                        e.hover = None;
                    }
                });
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_DESTROY => {
                with(|a| a.cities = None);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
