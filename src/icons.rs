//! Vector icons, drawn from the same shapes as the old SVGs.

use crate::{
    gfx::{mul, scale, translate, Color, Gfx, Rect},
    weather::Kind,
};

const SUN: Color = Color::hex(0xffd60a);
const MOON: &str = "M60 18a32 32 0 1 0 24 50A26 26 0 0 1 60 18z";
const BOLT: &str = "M55 64 41 84h11l-6 15 18-22H52l6-13z";

fn cloud(g: &Gfx, fill: Color) {
    g.fill_circle(34.0, 58.0, 15.0, fill);
    g.fill_circle(53.0, 46.0, 21.0, fill);
    g.fill_circle(71.0, 59.0, 14.0, fill);
    g.fill_rrect(Rect::new(20.0, 56.0, 64.0, 17.0), 8.5, fill);
}

fn sun(g: &Gfx) {
    let r = 17.0f32;
    for i in 0..8 {
        let a = i as f32 * std::f32::consts::FRAC_PI_4;
        let p = |d: f32| (50.0 + a.cos() * d, 50.0 + a.sin() * d);
        let (p0, p1) = (p(r + 9.0), p(r + 17.0));
        g.line(p0.0, p0.1, p1.0, p1.1, 6.0, SUN, true);
    }
    g.fill_circle(50.0, 50.0, r, SUN);
}

fn moon(g: &Gfx) {
    g.fill_path(MOON, Color::hex(0xfde9a6));
}

fn drops(g: &Gfx, c: Color) {
    for x in [37.0, 53.0, 69.0] {
        g.line(x, 78.0, x - 4.0, 89.0, 5.0, c, true);
    }
}

/// translate(tx ty) scale(s) translate(-50 -50) — the SVG "small sun/moon in the corner" group.
fn corner(g: &Gfx, tx: f32, ty: f32, s: f32, f: impl FnOnce(&Gfx)) {
    g.push_transform(mul(mul(translate(-50.0, -50.0), scale(s)), translate(tx, ty)));
    f(g);
    g.pop_transform();
}

fn shifted(g: &Gfx, dx: f32, dy: f32, f: impl FnOnce(&Gfx)) {
    g.push_transform(translate(dx, dy));
    f(g);
    g.pop_transform();
}

/// Weather icon in a `size`×`size` box at (x, y).
pub fn weather(g: &Gfx, kind: Kind, x: f32, y: f32, size: f32) {
    g.push_transform(mul(scale(size / 100.0), translate(x, y)));
    let up = |g: &Gfx, fill: Color| shifted(g, 0.0, -12.0, |g| cloud(g, fill));
    match kind {
        Kind::ClearDay => sun(g),
        Kind::ClearNight => moon(g),
        Kind::PartlyDay => {
            corner(g, 62.0, 32.0, 0.62, sun);
            shifted(g, -4.0, 8.0, |g| cloud(g, Color::hex(0xffffff)));
        }
        Kind::PartlyNight => {
            corner(g, 64.0, 30.0, 0.55, moon);
            shifted(g, -4.0, 8.0, |g| cloud(g, Color::hex(0xe9edf3)));
        }
        Kind::Cloudy => shifted(g, 0.0, 2.0, |g| cloud(g, Color::hex(0xeef1f5))),
        Kind::Fog => {
            let c = Color::hex(0xe3e7ed);
            up(g, c);
            g.line(24.0, 78.0, 76.0, 78.0, 5.0, c.alpha(0.8), true);
            g.line(32.0, 89.0, 72.0, 89.0, 5.0, c.alpha(0.8), true);
        }
        Kind::Drizzle => {
            up(g, Color::hex(0xeef1f5));
            drops(g, Color::hex(0x7fd4ff));
        }
        Kind::Rain => {
            up(g, Color::hex(0xdde2e9));
            drops(g, Color::hex(0x4fc3ff));
        }
        Kind::Snow => {
            up(g, Color::hex(0xeef1f5));
            for (cx, cy) in [(35.0, 83.0), (52.0, 89.0), (69.0, 83.0)] {
                g.fill_circle(cx, cy, 4.0, Color::white(1.0));
            }
        }
        Kind::Sleet => {
            up(g, Color::hex(0xdde2e9));
            let c = Color::hex(0x4fc3ff);
            g.line(37.0, 78.0, 33.0, 89.0, 5.0, c, true);
            g.line(69.0, 78.0, 65.0, 89.0, 5.0, c, true);
            g.fill_circle(52.0, 86.0, 4.0, Color::white(1.0));
        }
        Kind::Thunder => {
            up(g, Color::hex(0xd3d9e1));
            g.fill_path(BOLT, SUN);
        }
    }
    g.pop_transform();
}

// ---------- UI icons (24×24 viewBox) ----------

pub const PREV: &str = "M6 5h2v14H6zM20 5v14L9.5 12z";
pub const NEXT: &str = "M16 5h2v14h-2zM4 5v14l10.5-7z";
pub const PLAY: &str = "M7 4.5v15l13-7.5z";
pub const PAUSE: &str = "M6 4.5h4.2v15H6zM13.8 4.5H18v15h-4.2z";
pub const NOTE: &str =
    "M9 17.5a2.5 2.5 0 1 1-2.5-2.5c.36 0 .7.07 1 .2V5.5l11-2.5v12a2.5 2.5 0 1 1-2.5-2.5c.36 0 .7.07 1 .2V7.3l-8 1.8z";
pub const ARROW: &str = "M21 3 3 10.5l7.5 3L13.5 21z";

/// Filled 24-unit icon path in a `size` box.
pub fn glyph(g: &Gfx, d: &'static str, x: f32, y: f32, size: f32, c: Color) {
    g.push_transform(mul(scale(size / 24.0), translate(x, y)));
    g.fill_path(d, c);
    g.pop_transform();
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mini {
    Cpu,
    Ram,
    Gpu,
    Disk,
}

const CPU_PINS: &str = "M9 3v3M15 3v3M9 18v3M15 18v3M3 9h3M3 15h3M18 9h3M18 15h3";
const RAM_PINS: &str = "M7 10v4M11 10v4M15 10v4M6 17v2.5M10 17v2.5M14 17v2.5M18 17v2.5";
const GPU_LINES: &str = "M15 9.5h3.5M15 13.5h3.5M5 17v2.5";
const DISK_LINE: &str = "M7 15h5";

/// Stroke icons of the system widget (stroke-width 1.7 in a 24-unit box).
pub fn mini(g: &Gfx, m: Mini, x: f32, y: f32, size: f32, c: Color) {
    g.push_transform(mul(scale(size / 24.0), translate(x, y)));
    let sw = 1.7;
    // SVG strokes are centred on the outline, so stroke_rrect matches `rect` + `stroke`.
    match m {
        Mini::Cpu => {
            g.stroke_rrect(Rect::new(6.0, 6.0, 12.0, 12.0), 2.5, sw, c);
            g.fill_rrect(Rect::new(9.5, 9.5, 5.0, 5.0), 1.0, c);
            g.stroke_path(CPU_PINS, sw, c);
        }
        Mini::Ram => {
            g.stroke_rrect(Rect::new(3.0, 7.0, 18.0, 10.0), 2.0, sw, c);
            g.stroke_path(RAM_PINS, sw, c);
        }
        Mini::Gpu => {
            g.stroke_rrect(Rect::new(2.5, 6.0, 19.0, 11.0), 2.0, sw, c);
            g.stroke_circle(9.0, 11.5, 3.0, sw, c);
            g.stroke_path(GPU_LINES, sw, c);
        }
        Mini::Disk => {
            g.stroke_rrect(Rect::new(3.5, 5.0, 17.0, 14.0), 2.5, sw, c);
            g.fill_circle(16.5, 15.0, 1.3, c);
            g.stroke_path(DISK_LINE, sw, c);
        }
    }
    g.pop_transform();
}
