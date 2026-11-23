//! Frosted glass: the desktop wallpaper, fitted to each monitor ("Fill"), at quarter
//! resolution, blurred (CSS `blur(34px) saturate(1.5)`) once. Widgets then just sample the
//! piece under themselves.

use std::path::PathBuf;

use windows::Win32::{
    Foundation::RECT,
    UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETDESKWALLPAPER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS},
};

use crate::image::{self, Image};

/// Work at a quarter of the monitor resolution; it gets blurred anyway.
pub const SCALE: f64 = 0.25;
/// CSS px
const BLUR: f64 = 34.0;
const SATURATE: f32 = 1.5;

pub struct Wall {
    /// Monitor rectangle, physical px.
    pub mon: RECT,
    pub img: Image,
}

/// The current desktop wallpaper image file, if there is one.
pub fn path() -> Option<PathBuf> {
    let mut buf = [0u16; 1024];
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETDESKWALLPAPER,
            buf.len() as u32,
            Some(buf.as_mut_ptr() as _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if ok.is_ok() {
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

/// Build the blurred backdrop for every monitor. `mons`: (rect, dpi scale). Needs COM.
pub fn build(mons: &[(RECT, f32)]) -> Vec<Wall> {
    let Some(p) = path() else { return Vec::new() };
    mons.iter()
        .filter_map(|&(mon, sf)| {
            let w = ((mon.right - mon.left) as f64 * SCALE).round().max(1.0) as u32;
            let h = ((mon.bottom - mon.top) as f64 * SCALE).round().max(1.0) as u32;
            let mut img = image::decode_cover(image::Source::File(&p), w, h)?;
            let sigma = BLUR * sf as f64 * SCALE;
            blur(&mut img, sigma);
            saturate(&mut img, SATURATE);
            normalize(&mut img);
            Some(Wall { mon, img })
        })
        .collect()
}

/// Gaussian blur approximated by three box blurs (clamped edges).
fn blur(img: &mut Image, sigma: f64) {
    let n = 3.0;
    let w_ideal = (12.0 * sigma * sigma / n + 1.0).sqrt();
    let mut wl = w_ideal.floor() as i64;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wu = wl + 2;
    let m = ((12.0 * sigma * sigma - n * (wl * wl) as f64 - 4.0 * n * wl as f64 - 3.0 * n) / (-4.0 * wl as f64 - 4.0))
        .round() as i64;
    let (w, h) = (img.w as usize, img.h as usize);
    let mut tmp = vec![0u8; img.px.len()];
    for i in 0..3 {
        let r = (((if i < m { wl } else { wu }) - 1) / 2).max(0) as usize;
        if r == 0 {
            continue;
        }
        box_pass(&img.px, &mut tmp, w, h, r, true);
        box_pass(&tmp, &mut img.px, w, h, r, false);
    }
}

fn box_pass(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize, horiz: bool) {
    let (len, lines) = if horiz { (w, h) } else { (h, w) };
    let idx = |line: usize, i: usize| if horiz { (line * w + i) * 4 } else { (i * w + line) * 4 };
    let div = (2 * r + 1) as u32;
    for line in 0..lines {
        let mut acc = [0u32; 4];
        let at = |i: isize| idx(line, i.clamp(0, len as isize - 1) as usize);
        for i in -(r as isize)..=(r as isize) {
            let p = at(i);
            for c in 0..4 {
                acc[c] += src[p + c] as u32;
            }
        }
        for i in 0..len {
            let o = idx(line, i);
            for c in 0..4 {
                dst[o + c] = ((acc[c] + div / 2) / div) as u8;
            }
            let add = at(i as isize + r as isize + 1);
            let sub = at(i as isize - r as isize);
            for c in 0..4 {
                acc[c] = acc[c] + src[add + c] as u32 - src[sub + c] as u32;
            }
        }
    }
}

/// Make the glass look the same on any wallpaper: brightness is mapped into a fixed dark
/// range and colour is toned down, so only a hint of the wallpaper's hue and texture stays.
fn normalize(img: &mut Image) {
    const BASE: f32 = 0.11; // luminance of a black wallpaper
    const RANGE: f32 = 0.07; // extra luminance for a white one
    const TINT: f32 = 0.30; // share of the original colour kept
    for p in img.px.chunks_exact_mut(4) {
        let (b, g, r) = (p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
        let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let t = BASE + RANGE * lum;
        let map = |c: f32| ((t + (c - lum) * TINT).clamp(0.0, 1.0) * 255.0).round() as u8;
        (p[0], p[1], p[2]) = (map(b), map(g), map(r));
    }
}

/// CSS saturate() matrix.
fn saturate(img: &mut Image, s: f32) {
    for p in img.px.chunks_exact_mut(4) {
        let (b, g, r) = (p[0] as f32, p[1] as f32, p[2] as f32);
        let nr = (0.213 + 0.787 * s) * r + (0.715 - 0.715 * s) * g + (0.072 - 0.072 * s) * b;
        let ng = (0.213 - 0.213 * s) * r + (0.715 + 0.285 * s) * g + (0.072 - 0.072 * s) * b;
        let nb = (0.213 - 0.213 * s) * r + (0.715 - 0.715 * s) * g + (0.072 + 0.928 * s) * b;
        let a = p[3] as f32;
        p[0] = nb.clamp(0.0, a) as u8;
        p[1] = ng.clamp(0.0, a) as u8;
        p[2] = nr.clamp(0.0, a) as u8;
    }
}
