//! Grid layout for widgets: equal gaps, magnetic snapping after a drag, and
//! "stacked" widgets that follow when the one above them changes height.
//! Pure geometry on physical-pixel rectangles; the caller moves the windows.

use crate::Id;

/// Space between widgets and from the screen edge, logical px.
pub const GAP: f64 = 16.0;
pub const MARGIN: f64 = 24.0;
/// How close (logical px) an edge must be to snap to a neighbour or the screen edge.
const SNAP: f64 = 48.0;
/// Fallback grid step when nothing is close enough to snap to.
const GRID: f64 = 8.0;
/// Edges this close (physical px) count as touching.
const TOUCH: i32 = 3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    fn overlaps_x(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right()
    }
    /// Overlap including the required gap around each widget.
    fn collides(&self, o: &Rect, gap: i32) -> bool {
        self.x < o.right() + gap && o.x < self.right() + gap && self.y < o.bottom() + gap && o.y < self.bottom() + gap
    }
    pub fn from_win(r: &windows::Win32::Foundation::RECT) -> Rect {
        Rect { x: r.left, y: r.top, w: r.right - r.left, h: r.bottom - r.top }
    }
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

/// The user let go of a widget: align it to neighbours / screen edges, then make sure it does
/// not overlap anything. Returns the new top-left.
pub fn snap(mut me: Rect, others: &[Rect], wa: Rect, scale: f64) -> (i32, i32) {
    let px = |v: f64| (v * scale).round() as i32;
    let (gap, margin, limit, grid) = (px(GAP), px(MARGIN), px(SNAP), px(GRID).max(1));

    let mut xs = vec![wa.x + margin, wa.right() - margin - me.w];
    let mut ys = vec![wa.y + margin, wa.bottom() - margin - me.h];
    for o in others {
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
    (me.x, me.y)
}

/// Widget `id` changed height from `old` to `new_h`: everything stacked under it (top edge
/// exactly one gap below, overlapping horizontally) moves by the same amount.
pub fn reflow_below(id: Id, old: Rect, new_h: i32, all: &[(Id, Rect)], scale: f64) -> Vec<(Id, i32, i32)> {
    let delta = new_h - old.h;
    let mut out = Vec::new();
    if delta == 0 {
        return out;
    }
    let gap = (GAP * scale).round() as i32;
    let mut moved = vec![id];
    let mut queue = vec![old];
    while let Some(above) = queue.pop() {
        for (wid, r) in all {
            if moved.contains(wid) {
                continue;
            }
            if (r.y - (above.bottom() + gap)).abs() <= TOUCH && r.overlaps_x(&above) {
                moved.push(*wid);
                queue.push(*r); // its own followers sit below its old position
                out.push((*wid, r.x, r.y + delta));
            }
        }
    }
    out
}

/// Default stacking along the right edge of the work area: rows top-to-bottom, wrapping
/// into a new column to the left. `size(id)` gives the logical (w, h).
pub fn default_positions(
    work: Rect,
    scale: f64,
    ids: &[&[Id]],
    size: impl Fn(Id) -> (f64, f64),
) -> Vec<(Id, i32, i32)> {
    let mut out = Vec::new();
    let lw = work.w as f64 / scale;
    let lh = work.h as f64 / scale;
    let mut col_x = lw - MARGIN - 360.0;
    let mut y = MARGIN;
    for row in ids {
        let rh = row.iter().map(|id| size(*id).1).fold(0.0, f64::max);
        if y + rh > lh - MARGIN && y > MARGIN {
            col_x -= 360.0 + GAP;
            y = MARGIN;
        }
        let mut x = col_x;
        for id in row.iter() {
            out.push((*id, work.x + (x * scale) as i32, work.y + (y * scale) as i32));
            x += size(*id).0 + GAP;
        }
        y += rh + GAP;
    }
    out
}

/// Expanded system widget: same x, opens downwards; shifts up just enough when it would run
/// off the bottom of the screen. Returns the new y.
pub fn expanded_y(orig: Rect, h: i32, wa: Rect, scale: f64) -> i32 {
    let margin = (MARGIN * scale).round() as i32;
    let mut y = orig.y;
    if y + h > wa.bottom() - margin {
        y = (wa.bottom() - margin - h).max(wa.y + margin);
    }
    y
}
