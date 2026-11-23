//! What each widget looks like. Geometry mirrors the old widget.css (px = logical px).

use std::collections::HashMap;

use windows::Win32::{Foundation::SYSTEMTIME, Graphics::Direct2D::ID2D1Bitmap};
use windows_numerics::Matrix3x2;

use crate::{
    gfx::{v2, Align, Color, Gfx, Rect},
    icons::{self, Mini},
    media::MediaState,
    settings::City,
    system::Stats,
    weather::{self, Forecast},
    Id,
};

pub const RADIUS: f32 = 22.0;
const FG1: Color = Color::white(1.0);
const FG2: Color = Color::white(0.72);
const FG3: Color = Color::white(0.46);
const RED: Color = Color::hex(0xff453a);
const ORANGE: Color = Color::hex(0xff9f0a);
const HAIR: Color = Color::white(0.13);

/// Clickable parts of a widget.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    Prev,
    Toggle,
    Next,
    /// Weather: city at this index of the list becomes the main one.
    City(usize),
    EditCities,
}

pub struct Data<'a> {
    pub now: SYSTEMTIME,
    pub unix_ms: f64,
    pub media: &'a MediaState,
    pub art: Option<&'a ID2D1Bitmap>,
    pub cities: &'a [City],
    pub weather: &'a HashMap<String, Forecast>,
    pub stats: Option<&'a Stats>,
    pub hover: Option<Hit>,
    pub pressed: Option<Hit>,
    pub expanded: bool,
}

pub type Zones = Vec<(Rect, Hit)>;

// ---------- card ----------

/// Glass (blurred wallpaper under the window) or a solid card, with the hairline edge.
pub fn card(g: &Gfx, w: f32, h: f32, glass: Option<(&ID2D1Bitmap, Matrix3x2)>) {
    let r = Rect::new(0.0, 0.0, w, h);
    match glass {
        Some((bmp, m)) => {
            g.fill_rrect_bitmap(r, RADIUS, bmp, m);
            g.fill_rrect_vgrad(r, RADIUS, Color::rgba(40, 40, 46, 0.38), Color::rgba(18, 18, 22, 0.52));
        }
        None => g.fill_rrect(r, RADIUS, Color::rgba(30, 30, 34, 0.94)),
    }
    g.stroke_rrect(r.inset(0.5), RADIUS - 0.5, 1.0, Color::white(0.12));
    g.inset_top_highlight(r, RADIUS, Color::white(0.10));
}

/// Logical height the widget wants right now.
pub fn height(g: &Gfx, id: Id, d: &Data) -> f32 {
    match id {
        Id::Weather => weather_height(g, d),
        Id::System if d.expanded => system_full_height(g, d),
        _ => id.size().1 as f32,
    }
}

pub fn render(g: &Gfx, id: Id, d: &Data, w: f32, h: f32, z: &mut Zones) {
    match id {
        Id::Clock => clock(g, d, h),
        Id::Calendar => calendar(g, d),
        Id::Media => media(g, d, z),
        Id::Weather => weather_w(g, d, w, z),
        Id::System => {
            if d.expanded {
                system_full(g, d)
            } else {
                system_mini(g, d)
            }
        }
    }
}

// ---------- dates ----------

const WEEKDAYS: [&str; 7] = ["воскресенье", "понедельник", "вторник", "среда", "четверг", "пятница", "суббота"];
const MONTHS: [&str; 12] = [
    "январь",
    "февраль",
    "март",
    "апрель",
    "май",
    "июнь",
    "июль",
    "август",
    "сентябрь",
    "октябрь",
    "ноябрь",
    "декабрь",
];
const MONTHS_GEN: [&str; 12] = [
    "января",
    "февраля",
    "марта",
    "апреля",
    "мая",
    "июня",
    "июля",
    "августа",
    "сентября",
    "октября",
    "ноября",
    "декабря",
];

fn days_in_month(y: i32, m: u16) -> u16 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        _ => 28,
    }
}

// ---------- clock ----------

fn clock(g: &Gfx, d: &Data, h: f32) {
    let t = &d.now;
    let lh_wd = g.normal_lh(12.0);
    let lh_date = g.normal_lh(14.0);
    let lh_year = g.normal_lh(12.0);
    let free = ((h - 27.0) - (lh_wd + 54.0 + lh_date + 1.0 + lh_year)).max(0.0) / 2.0;
    let y_time = 13.0 + lh_wd + free;
    let y_date = y_time + 54.0 + free;

    let wd = WEEKDAYS[t.wDayOfWeek as usize % 7].to_uppercase();
    g.text(&wd, 12.0).weight(700).ls(0.6).color(RED).draw(16.0, 13.0);
    let time = format!("{:02}:{:02}", t.wHour, t.wMinute);
    g.text(&time, 54.0).weight(300).lh(54.0).ls(-1.0).draw(14.0, y_time);
    let date = format!("{} {}", t.wDay, MONTHS_GEN[(t.wMonth as usize + 11) % 12]);
    g.text(&date, 14.0).color(FG2).draw(16.0, y_date);
    g.text(&t.wYear.to_string(), 12.0).color(FG3).draw(16.0, y_date + lh_date + 1.0);
}

// ---------- calendar ----------

fn calendar(g: &Gfx, d: &Data) {
    let t = &d.now;
    let month = MONTHS[(t.wMonth as usize + 11) % 12].to_uppercase();
    g.text(&month, 12.0).weight(700).ls(0.6).color(RED).draw(16.0, 13.0);

    let top = 13.0 + g.normal_lh(12.0) + 6.0;
    let (x0, gw) = (14.0, 172.0 - 14.0 - 13.0);
    let cw = gw / 7.0;
    let cell = |i: usize| (x0 + (i % 7) as f32 * cw, top + (i / 7) as f32 * 19.0);
    for (i, hd) in ["П", "В", "С", "Ч", "П", "С", "В"].iter().enumerate() {
        let (x, y) = cell(i);
        g.text(hd, 9.5).weight(700).color(FG3).lh(18.0).width(cw).align(Align::Center).draw(x, y);
    }
    // Monday-first offset of the 1st of the month.
    let first_dow = (t.wDayOfWeek as i32 - (t.wDay as i32 - 1)).rem_euclid(7);
    let lead = ((first_dow + 6) % 7) as usize;
    for day in 1..=days_in_month(t.wYear as i32, t.wMonth) {
        let i = 7 + lead + day as usize - 1;
        let (x, y) = cell(i);
        let col = i % 7;
        let s = day.to_string();
        if day == t.wDay {
            g.fill_circle(x + cw / 2.0, y + 9.0, 9.5, RED);
            g.text(&s, 11.0).weight(700).lh(18.0).width(cw).align(Align::Center).draw(x, y);
        } else {
            let c = if col >= 5 { FG2 } else { FG1 };
            g.text(&s, 11.0).weight(500).color(c).lh(18.0).width(cw).align(Align::Center).draw(x, y);
        }
    }
}

// ---------- now playing ----------

fn mmss(sec: f64) -> String {
    let s = sec.max(0.0).floor() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn media(g: &Gfx, d: &Data, z: &mut Zones) {
    let st = d.media;
    let on = st.active;

    // cover
    let art = Rect::new(14.0, 14.0, 122.0, 122.0);
    g.shadow(art, 12.0, 4.0, 14.0, Color(0.0, 0.0, 0.0, 0.35));
    match (on, d.art) {
        (true, Some(bmp)) => {
            g.push_clip_rrect(art, 12.0);
            g.draw_bitmap(bmp, art);
            g.pop_layer();
        }
        _ => {
            g.fill_rrect(art, 12.0, Color(0.11, 0.11, 0.12, 1.0));
            g.fill_rrect_grad(
                art,
                12.0,
                v2(art.x, art.y),
                v2(art.x + art.w, art.y + art.h),
                Color::white(0.10),
                Color::white(0.03),
            );
            g.stroke_rrect(art.inset(0.5), 11.5, 1.0, Color::white(0.08));
            icons::glyph(g, icons::NOTE, art.x + 39.0, art.y + 39.0, 44.0, FG3);
        }
    }

    // text
    let (mx, mw) = (150.0, 196.0);
    let src_h = if on && !st.source.is_empty() { g.normal_lh(11.0).max(14.0) } else { 14.0 };
    if on {
        g.text(&st.source.to_uppercase(), 11.0).weight(700).ls(0.5).color(RED).width(mw).ellipsis().draw(mx, 14.0);
    }
    let ty = 14.0 + src_h + 3.0;
    let (title, artist) = if on {
        let t = if st.title.is_empty() { "Без названия".to_string() } else { st.title.clone() };
        let a = if st.artist.is_empty() { st.album.clone() } else { st.artist.clone() };
        (t, a)
    } else {
        ("Ничего не играет".to_string(), "Включите музыку в любом плеере".to_string())
    };
    g.text(&title, 15.0)
        .weight(if on { 600 } else { 500 })
        .lh(19.5)
        .color(if on { FG1 } else { FG2 })
        .width(mw)
        .ellipsis()
        .draw(mx, ty);
    g.text(&artist, 12.5).lh(16.25).color(if on { FG2 } else { FG3 }).width(mw).ellipsis().draw(mx, ty + 19.5 + 1.0);

    // progress
    if on && st.duration > 0.0 {
        let mut pos = st.position;
        if st.playing && st.updated_at > 0.0 {
            pos += (d.unix_ms - st.updated_at) / 1000.0;
        }
        let pos = pos.clamp(0.0, st.duration);
        let lh_t = g.normal_lh(10.5);
        let times_y = 100.0 - lh_t;
        let bar_y = times_y - 3.0 - 4.0;
        g.fill_rrect(Rect::new(mx, bar_y, mw, 4.0), 2.0, Color::white(0.16));
        let fw = (mw as f64 * pos / st.duration) as f32;
        if fw > 0.5 {
            g.fill_rrect(Rect::new(mx, bar_y, fw, 4.0), 2.0, FG1);
        }
        g.text(&mmss(pos), 10.5).color(FG3).draw(mx, times_y);
        g.text(&mmss(st.duration), 10.5).color(FG3).width(mw).align(Align::Right).draw(mx, times_y);
    }

    // controls
    let playing = on && st.playing;
    let buttons = [
        (Hit::Prev, 196.0, 13.0, 18.0, icons::PREV, on && st.can_prev),
        (Hit::Toggle, 248.0, 17.0, 26.0, if playing { icons::PAUSE } else { icons::PLAY }, on),
        (Hit::Next, 300.0, 13.0, 18.0, icons::NEXT, on && st.can_next),
    ];
    for (hit, cx, r, size, path, enabled) in buttons {
        let cy = 120.0;
        if enabled {
            if d.pressed == Some(hit) {
                g.fill_circle(cx, cy, r * 0.9, Color::white(0.16));
            } else if d.hover == Some(hit) {
                g.fill_circle(cx, cy, r, Color::white(0.10));
            }
            z.push((Rect::new(cx - r, cy - r, 2.0 * r, 2.0 * r), hit));
        }
        let s = if d.pressed == Some(hit) && enabled { size * 0.9 } else { size };
        icons::glyph(g, path, cx - s / 2.0, cy - s / 2.0, s, FG1.alpha(if enabled { 1.0 } else { 0.3 }));
    }
}

// ---------- weather ----------

const TOP_H: f32 = 170.0;

fn row_height(g: &Gfx) -> f32 {
    let info = g.normal_lh(14.0) + 1.0 + g.normal_lh(11.5);
    let tv = 22.0 * 1.05 + g.normal_lh(10.5);
    16.0 + info.max(26.0).max(tv)
}

/// The link sits in a block whose line box keeps the 16 px body font's strut.
fn foot_height(g: &Gfx) -> f32 {
    4.0 + g.normal_lh(16.0) + 10.0
}

fn weather_height(g: &Gfx, d: &Data) -> f32 {
    let others = d.cities.len().saturating_sub(1);
    let list = if others > 0 { 1.0 + 4.0 + others as f32 * row_height(g) + (others - 1) as f32 } else { 0.0 };
    (TOP_H + list + foot_height(g)).round()
}

fn weather_w(g: &Gfx, d: &Data, w: f32, z: &mut Zones) {
    let (x0, x1) = (16.0, w - 16.0);
    let deg = weather::deg;
    match d.cities.first() {
        None => {
            let msg = "Добавьте город через меню «Города…»";
            let lh = g.normal_lh(13.0);
            g.text(msg, 13.0).color(FG2).width(x1 - x0).align(Align::Center).draw(x0, 14.0 + (144.0 - lh) / 2.0);
        }
        Some(city) => {
            let lh15 = g.normal_lh(15.0);
            let nw = g.text(&city.name, 15.0).weight(600).width(260.0).ellipsis().draw(x0, 14.0).min(260.0);
            icons::glyph(g, icons::ARROW, x0 + nw + 4.0, 14.0 + (lh15 - 11.0) / 2.0, 11.0, FG1);
            match d.weather.get(&city.key()) {
                None => {
                    let lh = g.normal_lh(13.0);
                    g.text("Загрузка…", 13.0)
                        .color(FG2)
                        .width(x1 - x0)
                        .align(Align::Center)
                        .draw(x0, 14.0 + (144.0 - lh) / 2.0 + 8.0);
                }
                Some(f) => {
                    g.text(&deg(f.temp), 46.0).weight(300).lh(46.0).draw(x0 - 2.0, 14.0 + lh15 + 2.0);
                    icons::weather(g, weather::kind(f.code, f.is_day), x1 - 24.0, 14.0, 24.0);
                    let lh = g.normal_lh(12.5);
                    let rw = 220.0;
                    g.text(weather::text(f.code), 12.5)
                        .weight(600)
                        .width(rw)
                        .align(Align::Right)
                        .draw(x1 - rw, 14.0 + 27.0);
                    let hilo = format!("Макс.: {} Мин.: {}", deg(f.max), deg(f.min));
                    g.text(&hilo, 12.5).weight(600).width(rw).align(Align::Right).draw(x1 - rw, 14.0 + 27.0 + lh);

                    // next 6 hours
                    let lt = g.normal_lh(11.0);
                    let hh = lt + 3.0 + 24.0 + 2.0 + g.normal_lh(13.0);
                    let hy = TOP_H - 12.0 - hh;
                    let cw = (x1 - x0) / 6.0;
                    let now = f.time.get(..13).unwrap_or("");
                    let start = f.hourly.iter().position(|h| h.time.get(..13) == Some(now)).unwrap_or(0);
                    for (k, h) in f.hourly.iter().skip(start).take(6).enumerate() {
                        let cx = x0 + k as f32 * cw;
                        let label = if k == 0 {
                            "Сейчас".to_string()
                        } else {
                            h.time.get(11..13).unwrap_or("").to_string()
                        };
                        g.text(&label, 11.0).weight(600).color(FG2).width(cw).align(Align::Center).draw(cx, hy);
                        icons::weather(g, weather::kind(h.code, h.is_day), cx + (cw - 24.0) / 2.0, hy + lt + 3.0, 24.0);
                        g.text(&deg(h.temp), 13.0).weight(600).width(cw).align(Align::Center).draw(cx, hy + lt + 29.0);
                    }
                }
            }
        }
    }

    // other cities
    let mut y = TOP_H;
    if d.cities.len() > 1 {
        g.fill_rect(Rect::new(10.0, y, w - 20.0, 1.0), Color::white(0.22));
        y += 5.0;
        let rh = row_height(g);
        for (i, c) in d.cities.iter().enumerate().skip(1) {
            let border = if i > 1 { 1.0 } else { 0.0 };
            let row = Rect::new(10.0, y, w - 20.0, rh + border);
            let hit = Hit::City(i);
            if d.pressed == Some(hit) {
                g.fill_rrect(row, 10.0, Color::white(0.16));
            } else if d.hover == Some(hit) {
                g.fill_rrect(row, 10.0, Color::white(0.10));
            } else if border > 0.0 {
                g.fill_rect(Rect::new(row.x, row.y, row.w, 1.0), Color::white(0.12));
            }
            z.push((row, hit));
            let cy = y + border + rh / 2.0; // vertical centre of the row content
            let (ix0, ix1) = (16.0, w - 16.0);
            let lh_n = g.normal_lh(14.0);
            let lh_s = g.normal_lh(11.5);
            let info_top = cy - (lh_n + 1.0 + lh_s) / 2.0;
            match d.weather.get(&c.key()) {
                None => {
                    g.text(&c.name, 14.0).weight(600).width(ix1 - ix0).ellipsis().draw(ix0, info_top);
                    g.text("Загрузка…", 11.5).color(FG2).draw(ix0, info_top + lh_n + 1.0);
                }
                Some(f) => {
                    let t = deg(f.temp);
                    let hl = format!("↑{} ↓{}", deg(f.max), deg(f.min));
                    let tw = g.text(&t, 22.0).weight(300).measure();
                    let hw = g.text(&hl, 10.5).weight(600).measure();
                    let tvw = tw.max(hw).max(56.0);
                    let tv_h = 22.0 * 1.05 + g.normal_lh(10.5);
                    let tv_top = cy - tv_h / 2.0;
                    g.text(&t, 22.0).weight(300).lh(22.0 * 1.05).width(tvw).align(Align::Right).draw(ix1 - tvw, tv_top);
                    g.text(&hl, 10.5)
                        .weight(600)
                        .color(FG2)
                        .width(tvw)
                        .align(Align::Right)
                        .draw(ix1 - tvw, tv_top + 22.0 * 1.05);
                    let icon_x = ix1 - tvw - 10.0 - 26.0;
                    icons::weather(g, weather::kind(f.code, f.is_day), icon_x, cy - 13.0, 26.0);
                    let iw = icon_x - 10.0 - ix0;
                    g.text(&c.name, 14.0).weight(600).width(iw).ellipsis().draw(ix0, info_top);
                    let sub = format!("{} · {}", weather::city_time(f), weather::text(f.code));
                    g.text(&sub, 11.5).color(FG2).width(iw).ellipsis().draw(ix0, info_top + lh_n + 1.0);
                }
            }
            y += rh + border;
        }
    }

    // footer link
    let fy = y + 4.0 + g.baseline(16.0) - g.baseline(11.5);
    let hover = d.hover == Some(Hit::EditCities);
    let ew = g.text("Изменить список городов…", 11.5).color(if hover { FG1 } else { FG2 }).draw(16.0, fy);
    z.push((Rect::new(16.0, fy, ew, g.normal_lh(11.5)), Hit::EditCities));
}

// ---------- system ----------

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

fn size_text(b: u64) -> String {
    let b = b as f64;
    if b >= 1024.0 * GIB {
        format!("{:.1} ТБ", b / 1024.0 / GIB)
    } else if b >= 100.0 * GIB {
        format!("{} ГБ", (b / GIB).round())
    } else {
        format!("{:.1} ГБ", b / GIB)
    }
}

fn speed(bps: f64) -> String {
    if bps >= 1024.0 * 1024.0 {
        format!("{:.1} МБ/с", bps / 1024.0 / 1024.0)
    } else {
        format!("{} КБ/с", (bps / 1024.0).round())
    }
}

fn uptime_text(sec: u64) -> String {
    let (d, h, m) = (sec / 86400, (sec % 86400) / 3600, (sec % 3600) / 60);
    if d > 0 {
        format!("{d} д {h} ч")
    } else if h > 0 {
        format!("{h} ч {m} мин")
    } else {
        format!("{m} мин")
    }
}

/// Monochrome UI: white while calm, orange/red only as a warning.
fn load_color(pct: f64) -> Color {
    if pct >= 95.0 {
        RED
    } else if pct >= 85.0 {
        ORANGE
    } else {
        FG1
    }
}
fn temp_color(t: u32) -> Color {
    if t >= 85 {
        RED
    } else if t >= 70 {
        ORANGE
    } else {
        FG1
    }
}

fn pct_text(p: f64) -> String {
    format!("{}%", (p + 0.5).floor() as i64)
}

fn ram_pct(s: &Stats) -> f64 {
    if s.ram_total > 0 {
        s.ram_used as f64 / s.ram_total as f64 * 100.0
    } else {
        0.0
    }
}

fn disk_pct(d: &crate::system::DiskInfo) -> f64 {
    d.total.saturating_sub(d.available) as f64 / d.total.max(1) as f64 * 100.0
}

fn system_mini(g: &Gfx, d: &Data) {
    let cw = 340.0 / 3.0;
    let total = 66.0 + 8.0 + g.normal_lh(11.5) + 1.0 + g.normal_lh(17.0);
    let top = 14.0 + (122.0 - total) / 2.0;
    let k = 66.0 / 64.0;
    let mut cells: Vec<(Mini, String, Option<f64>)> = vec![
        (Mini::Cpu, "Процессор".into(), d.stats.map(|s| s.cpu as f64)),
        (Mini::Ram, "Память".into(), d.stats.map(ram_pct)),
        (Mini::Gpu, "Видеокарта".into(), None),
    ];
    if let Some(s) = d.stats {
        if let Some(gpu) = s.gpus.first() {
            cells[2].2 = Some(gpu.load as f64);
        } else if let Some(c) = s.disks.iter().find(|x| x.mount.to_uppercase().starts_with('C')).or(s.disks.first()) {
            // No NVIDIA GPU: show the system drive instead.
            cells[2] = (Mini::Disk, format!("Диск {}", c.mount), Some(disk_pct(c)));
        }
    }
    for (i, (icon, label, pct)) in cells.iter().enumerate() {
        let cx = 10.0 + cw * i as f32 + cw / 2.0;
        let cy = top + 33.0;
        let p = pct.unwrap_or(0.0);
        let c = load_color(p);
        g.ring(cx, cy, 27.0 * k, 6.5 * k, p as f32, c, c.alpha(0.18));
        icons::mini(g, *icon, cx - 12.0, cy - 12.0, 24.0, c);
        let ly = top + 66.0 + 8.0;
        g.text(label, 11.5).weight(500).color(FG2).width(cw).align(Align::Center).draw(cx - cw / 2.0, ly);
        let v = pct.map(pct_text).unwrap_or_else(|| "–".into());
        g.text(&v, 17.0).weight(600).width(cw).align(Align::Center).draw(cx - cw / 2.0, ly + g.normal_lh(11.5) + 1.0);
    }
}

struct Block {
    pct: Option<f64>,
    title: &'static str,
    name: String,
    sub: String,
    temp: Option<u32>,
    has_temp: bool,
}

fn blocks(d: &Data) -> Vec<Block> {
    let mut v = Vec::new();
    let s = d.stats;
    v.push(Block {
        pct: s.map(|s| s.cpu as f64),
        title: "Процессор",
        name: s
            .map(|s| if s.cpu_name.is_empty() { "Процессор".into() } else { s.cpu_name.clone() })
            .unwrap_or_default(),
        sub: s
            .map(|s| {
                let ghz = if s.cpu_freq > 0 {
                    format!(" · {:.1} ГГц", s.cpu_freq as f64 / 1000.0)
                } else {
                    String::new()
                };
                format!("{} потоков{ghz}", s.threads)
            })
            .unwrap_or_default(),
        temp: None,
        has_temp: false,
    });
    v.push(Block {
        pct: s.map(ram_pct),
        title: "Память",
        name: s
            .map(|s| format!("{:.1} из {} ГБ", s.ram_used as f64 / GIB, (s.ram_total as f64 / GIB).round()))
            .unwrap_or_default(),
        sub: s.map(|s| format!("Свободно {:.1} ГБ", (s.ram_total - s.ram_used) as f64 / GIB)).unwrap_or_default(),
        temp: None,
        has_temp: false,
    });
    for gpu in s.map(|s| s.gpus.as_slice()).unwrap_or(&[]) {
        let mut parts =
            vec![format!("VRAM {:.1} из {} ГБ", gpu.mem_used as f64 / GIB, (gpu.mem_total as f64 / GIB).round())];
        if let Some(p) = gpu.power {
            parts.push(format!("мощн. {p}%"));
        }
        if let Some(f) = gpu.fan.filter(|f| *f > 0) {
            parts.push(format!("вент. {f}%"));
        }
        v.push(Block {
            pct: Some(gpu.load as f64),
            title: "Видеокарта",
            name: gpu.name.clone(),
            sub: parts.join(" · "),
            temp: gpu.temp,
            has_temp: true,
        });
    }
    v
}

fn disk_height(g: &Gfx) -> f32 {
    6.0 + g.normal_lh(13.0) + 5.0 + 6.0 + 3.0 + g.normal_lh(11.0) + 6.0
}

fn system_full_height(g: &Gfx, d: &Data) -> f32 {
    let n = blocks(d).len() as f32;
    let disks = d.stats.map(|s| s.disks.len()).unwrap_or(0);
    let main = n * 48.0 + (n - 1.0) * 10.0;
    let net = 12.0 + 1.0 + 8.0 + g.normal_lh(12.5) + 8.0 + 1.0;
    let dh = 10.0 + g.normal_lh(11.0) + 2.0;
    let list = if disks > 0 { disks as f32 * disk_height(g) } else { g.normal_lh(11.0) };
    (14.0 + g.normal_lh(15.0) + 10.0 + main + net + dh + list + 14.0).ceil()
}

fn system_full(g: &Gfx, d: &Data) {
    let (x0, x1) = (16.0, 344.0);
    let asc = |size: f32| g.baseline(size);
    // header (baseline-aligned)
    g.text("Система", 15.0).weight(600).draw(x0, 14.0);
    if let Some(s) = d.stats {
        let up = format!("Работает {}", uptime_text(s.uptime));
        let rw = 200.0;
        g.text(&up, 11.5).color(FG2).width(rw).align(Align::Right).draw(x1 - rw, 14.0 + asc(15.0) - asc(11.5));
    }
    let mut y = 14.0 + g.normal_lh(15.0) + 10.0;

    let k = 48.0 / 52.0;
    for b in blocks(d) {
        let p = b.pct.unwrap_or(0.0);
        let c = load_color(p);
        let (cx, cy) = (x0 + 24.0, y + 24.0);
        g.ring(cx, cy, 22.0 * k, 6.0 * k, p as f32, c, c.alpha(0.2));
        let v = b.pct.map(pct_text).unwrap_or_else(|| "–".into());
        let lh12 = g.normal_lh(12.0);
        g.text(&v, 12.0).weight(600).width(48.0).align(Align::Center).draw(x0, cy - lh12 / 2.0);

        let mut tx1 = x1;
        if b.has_temp {
            if let Some(t) = b.temp {
                let ts = format!("{t}°");
                let lh = g.normal_lh(22.0);
                let tw = g.text(&ts, 22.0).weight(300).measure();
                g.text(&ts, 22.0).weight(300).color(temp_color(t)).draw(x1 - tw, cy - lh / 2.0);
                tx1 = x1 - tw - 12.0;
            }
        }
        let tx = x0 + 48.0 + 12.0;
        let tw = tx1 - tx;
        let th = 11.0 * 1.3 + 13.0 * 1.3 + 11.5 * 1.3;
        let ty = cy - th / 2.0;
        g.text(&b.title.to_uppercase(), 11.0)
            .weight(600)
            .ls(0.4)
            .color(FG2)
            .lh(11.0 * 1.3)
            .width(tw)
            .ellipsis()
            .draw(tx, ty);
        g.text(&b.name, 13.0).weight(600).lh(13.0 * 1.3).width(tw).ellipsis().draw(tx, ty + 11.0 * 1.3);
        g.text(&b.sub, 11.5).color(FG2).lh(11.5 * 1.3).width(tw).ellipsis().draw(tx, ty + 11.0 * 1.3 + 13.0 * 1.3);
        y += 48.0 + 10.0;
    }
    y -= 10.0;

    // network
    y += 12.0;
    g.fill_rect(Rect::new(x0, y, x1 - x0, 1.0), HAIR);
    let lh = g.normal_lh(12.5);
    let ly = y + 1.0 + 8.0;
    let lh11 = g.normal_lh(11.0);
    g.text("СЕТЬ", 11.0).weight(600).ls(0.4).color(FG2).draw(x0, ly + (lh - lh11) / 2.0);
    if let Some(s) = d.stats {
        let item = |arrow: &str, val: &str, right: f32| -> f32 {
            let a = format!("{arrow} ");
            let aw = g.text(&a, 12.5).color(FG2).measure();
            let vw = g.text(val, 12.5).weight(600).measure();
            let x = right - aw - vw;
            g.text(&a, 12.5).color(FG2).draw(x, ly);
            g.text(val, 12.5).weight(600).draw(x + aw, ly);
            x
        };
        let up = speed(s.net_up);
        let dn = speed(s.net_down);
        let ux = item("↑", &up, x1);
        item("↓", &dn, ux - 14.0);
    }
    y = ly + lh + 8.0;
    g.fill_rect(Rect::new(x0, y, x1 - x0, 1.0), HAIR);
    y += 1.0;

    // disks
    y += 10.0;
    g.text("ДИСКИ", 11.0).weight(600).ls(0.4).color(FG2).draw(x0, y);
    y += g.normal_lh(11.0) + 2.0;
    let disks = d.stats.map(|s| s.disks.as_slice()).unwrap_or(&[]);
    if disks.is_empty() {
        g.text("Диски не найдены", 11.0).color(FG2).draw(x0, y);
    }
    for disk in disks {
        let row = y + 6.0;
        let lh13 = g.normal_lh(13.0);
        let base = row + asc(13.0);
        let pct = disk_pct(disk);
        let pt = pct_text(pct);
        let pw = g.text(&pt, 12.5).weight(600).measure();
        g.text(&pt, 12.5).weight(600).draw(x1 - pw, base - asc(12.5));
        let mw = g.text(&disk.mount, 13.0).weight(700).draw(x0, row);
        let tag = if disk.removable { "USB" } else { disk.kind };
        let mut lab_end = x1 - pw - 6.0;
        if !tag.is_empty() {
            let tw = g.text(tag, 9.5).weight(700).measure() + 8.0 + 2.0;
            let tx = lab_end - tw;
            let top = base - (1.0 + g.baseline_in(9.5, 14.0));
            g.stroke_rrect(Rect::new(tx + 0.5, top + 0.5, tw - 1.0, 15.0), 4.0, 1.0, HAIR);
            g.text(tag, 9.5).weight(700).color(FG2).lh(14.0).draw(tx + 5.0, top + 1.0);
            lab_end = tx - 6.0;
        }
        let label = if !disk.label.is_empty() {
            disk.label.as_str()
        } else if disk.removable {
            "Съёмный диск"
        } else {
            "Локальный диск"
        };
        let lx = x0 + mw + 6.0;
        g.text(label, 12.0).color(FG2).width(lab_end - lx).ellipsis().draw(lx, base - asc(12.0));
        let by = row + lh13 + 5.0;
        g.fill_rrect(Rect::new(x0, by, x1 - x0, 6.0), 3.0, Color::white(0.12));
        let fw = ((x1 - x0) as f64 * pct / 100.0) as f32;
        if fw > 0.5 {
            g.fill_rrect(Rect::new(x0, by, fw, 6.0), 3.0, load_color(pct));
        }
        let ds = format!("{} свободно из {}", size_text(disk.available), size_text(disk.total));
        g.text(&ds, 11.0).color(FG2).draw(x0, by + 6.0 + 3.0);
        y += disk_height(g);
    }
}
