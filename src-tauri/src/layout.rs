//! Grid layout for widgets: equal gaps, magnetic snapping after a drag, and
//! "stacked" widgets that follow when the one above them changes height.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    time::Duration,
};

use tauri::{AppHandle, Manager, PhysicalPosition, WebviewWindow};

use crate::DEFS;

/// Space between widgets and from the screen edge, logical px.
pub const GAP: f64 = 16.0;
pub const MARGIN: f64 = 24.0;
/// How close (logical px) an edge must be to snap to a neighbour or the screen edge.
const SNAP: f64 = 48.0;
/// Fallback grid step when nothing is close enough to snap to.
const GRID: f64 = 8.0;
/// Edges this close (physical px) count as touching.
const TOUCH: i32 = 3;

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    fn right(&self) -> i32 {
        self.x + self.w
    }
    fn bottom(&self) -> i32 {
        self.y + self.h
    }
    fn overlaps_x(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right()
    }
    /// Overlap including the required gap around each widget.
    fn collides(&self, o: &Rect, gap: i32) -> bool {
        self.x < o.right() + gap && o.x < self.right() + gap && self.y < o.bottom() + gap && o.y < self.bottom() + gap
    }
}

fn rect_of(w: &WebviewWindow) -> Option<Rect> {
    let p = w.outer_position().ok()?;
    let s = w.outer_size().ok()?;
    Some(Rect { x: p.x, y: p.y, w: s.width as i32, h: s.height as i32 })
}

/// Every visible widget window and its rectangle (physical px).
fn widgets(app: &AppHandle) -> Vec<(WebviewWindow, Rect)> {
    DEFS.iter().filter_map(|d| app.get_webview_window(d.id)).filter_map(|w| rect_of(&w).map(|r| (w, r))).collect()
}

fn work_area(w: &WebviewWindow) -> Option<Rect> {
    let m = w.current_monitor().ok().flatten().or_else(|| w.primary_monitor().ok().flatten())?;
    let a = m.work_area();
    Some(Rect { x: a.position.x, y: a.position.y, w: a.size.width as i32, h: a.size.height as i32 })
}

/// Nearest candidate within `limit`, else the value rounded to the grid relative to `origin`.
fn snap_axis(value: i32, candidates: &[i32], limit: i32, origin: i32, grid: i32) -> i32 {
    candidates
        .iter()
        .copied()
        .filter(|c| (c - value).abs() <= limit)
        .min_by_key(|c| (c - value).abs())
        .unwrap_or_else(|| origin + ((value - origin) as f64 / grid as f64).round() as i32 * grid)
}

/// Called when the user lets go of a widget: align it to neighbours / screen edges, then
/// make sure it does not overlap anything.
pub fn snap(app: &AppHandle, id: &str) {
    let Some(win) = app.get_webview_window(id) else { return };
    let (Some(mut me), Some(wa), Ok(scale)) = (rect_of(&win), work_area(&win), win.scale_factor()) else { return };
    let px = |v: f64| (v * scale).round() as i32;
    let (gap, margin, limit, grid) = (px(GAP), px(MARGIN), px(SNAP), px(GRID).max(1));
    let others: Vec<Rect> = widgets(app).into_iter().filter(|(w, _)| w.label() != id).map(|(_, r)| r).collect();

    let mut xs = vec![wa.x + margin, wa.right() - margin - me.w];
    let mut ys = vec![wa.y + margin, wa.bottom() - margin - me.h];
    for o in &others {
        xs.extend([o.x, o.right() + gap, o.x - gap - me.w, o.right() - me.w]);
        ys.extend([o.y, o.bottom() + gap, o.y - gap - me.h, o.bottom() - me.h]);
    }
    me.x = snap_axis(me.x, &xs, limit, wa.x + margin, grid);
    me.y = snap_axis(me.y, &ys, limit, wa.y + margin, grid);

    // Dropped onto another widget: slide down below it (repeat for chains).
    for _ in 0..others.len() + 1 {
        match others.iter().filter(|o| me.collides(o, gap)).max_by_key(|o| o.bottom()) {
            Some(o) => me.y = o.bottom() + gap,
            None => break,
        }
    }
    let _ = win.set_position(PhysicalPosition::new(me.x, me.y));
}

/// A widget changed height from `old` to `new_h`: move everything stacked under it
/// (top edge exactly one gap below, overlapping horizontally) by the same amount.
pub fn reflow_below(app: &AppHandle, id: &str, old: Rect, new_h: i32) {
    let delta = new_h - old.h;
    if delta == 0 {
        return;
    }
    let Ok(scale) = app.get_webview_window(id).map_or(Ok(1.0), |w| w.scale_factor()) else { return };
    let gap = (GAP * scale).round() as i32;
    let all = widgets(app);
    let mut moved: Vec<String> = vec![id.to_string()];
    let mut queue = vec![old];
    while let Some(above) = queue.pop() {
        for (w, r) in &all {
            if moved.iter().any(|m| m == w.label()) {
                continue;
            }
            if (r.y - (above.bottom() + gap)).abs() <= TOUCH && r.overlaps_x(&above) {
                moved.push(w.label().to_string());
                queue.push(*r); // its own followers sit below its old position
                let _ = w.set_position(PhysicalPosition::new(r.x, r.y + delta));
            }
        }
    }
}

pub fn current_rect(w: &WebviewWindow) -> Option<Rect> {
    rect_of(w)
}

// ---------- drag end detection ----------

fn generations() -> &'static Mutex<HashMap<String, u64>> {
    static G: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    G.get_or_init(Default::default)
}
static NEXT: AtomicU64 = AtomicU64::new(1);

pub fn mouse_down() -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    unsafe { GetAsyncKeyState(VK_LBUTTON as i32) as u16 & 0x8000 != 0 }
}

/// Window moved. Moves made by the user (mouse held) get snapped once the drag ends,
/// then `done` runs (saving the position); programmatic moves are left alone.
pub fn on_moved(app: &AppHandle, id: &'static str, done: fn(&AppHandle)) {
    if !mouse_down() {
        return;
    }
    let gen = NEXT.fetch_add(1, Ordering::Relaxed);
    generations().lock().unwrap().insert(id.into(), gen);
    let app = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(120));
        if generations().lock().unwrap().get(id) != Some(&gen) {
            return; // a newer move superseded this one
        }
        if !mouse_down() {
            let a = app.clone();
            let _ = app.run_on_main_thread(move || {
                snap(&a, id);
                done(&a);
            });
            return;
        }
    });
}

// ---------- hover expansion (system widget) ----------

struct Expansion {
    /// Compact rectangle to return to.
    orig: Rect,
    /// Where we put the expanded window; if it is elsewhere on collapse, the user dragged it.
    at: (i32, i32),
}

static EXPANDED: Mutex<Option<Expansion>> = Mutex::new(None);

pub fn is_expanded() -> bool {
    EXPANDED.lock().unwrap().is_some()
}

/// Grow the window to `height` (logical px) on top of its neighbours without moving them.
/// Opens downwards; shifts up just enough when it would run off the bottom of the screen.
pub fn expand(win: &WebviewWindow, height: f64) {
    let (Some(cur), Some(wa), Ok(scale)) = (rect_of(win), work_area(win), win.scale_factor()) else { return };
    let first;
    let (orig, y, h) = {
        let mut g = EXPANDED.lock().unwrap();
        first = g.is_none();
        let orig = g.as_ref().map_or(cur, |e| e.orig);
        let h = (height * scale).round() as i32;
        let margin = (MARGIN * scale).round() as i32;
        let mut y = orig.y;
        if y + h > wa.bottom() - margin {
            y = (wa.bottom() - margin - h).max(wa.y + margin);
        }
        *g = Some(Expansion { orig, at: (orig.x, y) });
        (orig, y, h)
    }; // lock released before touching the window (its Moved handler checks is_expanded)
    if first {
        let _ = win.set_always_on_bottom(false);
        let _ = win.set_always_on_top(true);
    }
    let _ = win.set_size(tauri::PhysicalSize::new(orig.w as u32, h as u32));
    let _ = win.set_position(PhysicalPosition::new(orig.x, y));
}

/// Back to the compact size at the original spot (or where the user dragged it).
pub fn collapse(win: &WebviewWindow, height: f64) {
    let Some(e) = EXPANDED.lock().unwrap().take() else { return };
    let scale = win.scale_factor().unwrap_or(1.0);
    let (x, y) = match rect_of(win) {
        Some(c) if (c.x, c.y) != e.at => (c.x, c.y),
        _ => (e.orig.x, e.orig.y),
    };
    let _ = win.set_size(tauri::PhysicalSize::new(e.orig.w as u32, (height * scale).round() as u32));
    let _ = win.set_position(PhysicalPosition::new(x, y));
    let _ = win.set_always_on_top(false);
    let _ = win.set_always_on_bottom(true);
}
