use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::SystemTime,
};

use serde::{Deserialize, Serialize};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, RunEvent, Theme, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent, Wry,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

mod layout;
mod media;
mod platform;
mod system;

struct Def {
    id: &'static str,
    title: &'static str,
    w: f64,
    h: f64,
}

const DEFS: &[Def] = &[
    Def { id: "clock", title: "Часы", w: 172.0, h: 170.0 },
    Def { id: "media", title: "Сейчас играет", w: 360.0, h: 150.0 },
    Def { id: "weather", title: "Погода", w: 360.0, h: 300.0 },
    Def { id: "calendar", title: "Календарь", w: 172.0, h: 170.0 },
    Def { id: "system", title: "Система", w: 360.0, h: 150.0 },
];

/// Default stacking: rows top-to-bottom, wrapping into a new column to the left.
const LAYOUT: &[&[&str]] = &[&["clock", "calendar"], &["media"], &["weather"], &["system"]];

use layout::{GAP, MARGIN};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct City {
    pub name: String,
    #[serde(default)]
    pub admin: String,
    #[serde(default)]
    pub country: String,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WidgetState {
    pub visible: bool,
    pub x: Option<i32>,
    pub y: Option<i32>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settings {
    pub cities: Vec<City>,
    #[serde(default)]
    pub widgets: HashMap<String, WidgetState>,
}

fn city(name: &str, country: &str, lat: f64, lon: f64) -> City {
    City { name: name.into(), admin: String::new(), country: country.into(), lat, lon }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            cities: vec![
                city("Москва", "Россия", 55.7558, 37.6173),
                city("Лондон", "Великобритания", 51.5074, -0.1278),
                city("Нью-Йорк", "США", 40.7128, -74.006),
                city("Токио", "Япония", 35.6895, 139.6917),
            ],
            widgets: HashMap::new(),
        }
    }
}

struct AppState {
    settings: Mutex<Settings>,
    path: PathBuf,
    /// Created lazily on the first poll so it does not delay the widgets appearing.
    monitor: OnceLock<system::Monitor>,
}

impl AppState {
    fn save(&self) {
        let json = serde_json::to_string_pretty(&*self.settings.lock().unwrap());
        if let (Ok(json), Some(dir)) = (json, self.path.parent()) {
            let _ = fs::create_dir_all(dir);
            let _ = fs::write(&self.path, json);
        }
    }
}

// ---------- commands ----------

#[tauri::command]
fn get_settings(state: tauri::State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn set_cities(app: AppHandle, state: tauri::State<AppState>, cities: Vec<City>) {
    state.settings.lock().unwrap().cities = cities;
    state.save();
    let _ = app.emit("settings-changed", ());
}

#[tauri::command]
async fn system_stats(state: tauri::State<'_, AppState>) -> Result<system::Stats, ()> {
    Ok(state.monitor.get_or_init(system::Monitor::new).stats())
}

/// Lets a widget with dynamic content size its window to fit; widgets stacked
/// underneath move along so the gaps stay equal.
#[tauri::command]
fn fit_height(app: AppHandle, window: WebviewWindow, height: f64) -> Result<(), String> {
    let width = def(window.label()).map(|d| d.w).unwrap_or(360.0);
    let old = layout::current_rect(&window);
    window.set_size(LogicalSize::new(width, height.ceil())).map_err(|e| e.to_string())?;
    if let (Some(old), Ok(size)) = (old, window.outer_size()) {
        layout::reflow_below(&app, window.label(), old, size.height as i32);
    }
    Ok(())
}

/// System widget hover: full version on top of the neighbours, compact otherwise.
#[tauri::command]
fn system_expand(window: WebviewWindow, expanded: bool, height: f64) {
    if expanded {
        layout::expand(&window, height);
    } else {
        layout::collapse(&window, height);
    }
}

/// Desktop wallpaper as a data URL; widgets blur the part beneath them to fake glass.
/// Every widget asks for it, so the encoded image is cached until the file changes.
#[tauri::command]
async fn wallpaper() -> Option<String> {
    use base64::Engine;
    static CACHE: Mutex<Option<(PathBuf, SystemTime, String)>> = Mutex::new(None);

    let path = platform::wallpaper_path()?;
    let modified = fs::metadata(&path).and_then(|m| m.modified()).ok()?;
    let mut cache = CACHE.lock().unwrap();
    if let Some((p, m, url)) = cache.as_ref() {
        if *p == path && *m == modified {
            return Some(url.clone());
        }
    }
    let bytes = fs::read(&path).ok()?;
    let mime = match bytes.get(..4) {
        Some([0x89, b'P', b'N', b'G']) => "image/png",
        Some([b'B', b'M', ..]) => "image/bmp",
        Some([b'R', b'I', b'F', b'F']) => "image/webp",
        _ => "image/jpeg",
    };
    let url = format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes));
    *cache = Some((path, modified, url.clone()));
    Some(url)
}

#[tauri::command]
fn widget_menu(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    let build = || -> tauri::Result<Menu<Wry>> {
        let id = window.label();
        let menu = Menu::new(&app)?;
        menu.append(&MenuItem::with_id(&app, format!("hide:{id}"), "Скрыть виджет", true, None::<&str>)?)?;
        menu.append(&MenuItem::with_id(&app, "cities", "Города…", true, None::<&str>)?)?;
        menu.append(&MenuItem::with_id(&app, "refresh", "Обновить", true, None::<&str>)?)?;
        menu.append(&MenuItem::with_id(&app, "reset", "Расставить заново", true, None::<&str>)?)?;
        menu.append(&PredefinedMenuItem::separator(&app)?)?;
        menu.append(&MenuItem::with_id(&app, "quit", "Выход", true, None::<&str>)?)?;
        Ok(menu)
    };
    let menu = build().map_err(|e| e.to_string())?;
    window.popup_menu(&menu).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_cities(app: AppHandle) {
    open_cities_window(&app);
}

// ---------- windows ----------

fn open_cities_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("cities-editor") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let _ = WebviewWindowBuilder::new(app, "cities-editor", WebviewUrl::App("cities.html".into()))
        .title("Города — Win11Widget")
        .inner_size(460.0, 600.0)
        .min_inner_size(380.0, 440.0)
        .theme(Some(Theme::Dark))
        .center()
        .build();
}

fn def(id: &str) -> Option<&'static Def> {
    DEFS.iter().find(|d| d.id == id)
}

/// Physical positions for every widget, stacked along the right edge of the primary
/// monitor's work area. Uses each window's real height when it already exists, so the
/// gaps come out equal.
fn default_positions(app: &AppHandle) -> HashMap<&'static str, (i32, i32)> {
    let mut out = HashMap::new();
    let Some(mon) = app.primary_monitor().ok().flatten() else { return out };
    let scale = mon.scale_factor();
    let area = mon.work_area();
    let origin = area.position;
    let lw = area.size.width as f64 / scale;
    let lh = area.size.height as f64 / scale;
    let height = |d: &Def| {
        app.get_webview_window(d.id).and_then(|w| w.outer_size().ok()).map(|s| s.height as f64 / scale).unwrap_or(d.h)
    };

    let mut col_x = lw - MARGIN - 360.0;
    let mut y = MARGIN;
    for row in LAYOUT {
        let rh = row.iter().filter_map(|id| def(id)).map(height).fold(0.0, f64::max);
        if y + rh > lh - MARGIN && y > MARGIN {
            col_x -= 360.0 + GAP;
            y = MARGIN;
        }
        let mut x = col_x;
        for id in row.iter() {
            if let Some(d) = def(id) {
                out.insert(d.id, (origin.x + (x * scale) as i32, origin.y + (y * scale) as i32));
                x += d.w + GAP;
            }
        }
        y += rh + GAP;
    }
    out
}

fn on_screen(app: &AppHandle, x: i32, y: i32) -> bool {
    app.available_monitors().unwrap_or_default().iter().any(|m| {
        let p = m.position();
        let s = m.size();
        x >= p.x - 40 && y >= p.y - 40 && x < p.x + s.width as i32 - 40 && y < p.y + s.height as i32 - 40
    })
}

fn create_widget(app: &AppHandle, d: &Def) -> tauri::Result<()> {
    if app.get_webview_window(d.id).is_some() {
        return Ok(());
    }
    let state = app.state::<AppState>();
    let saved = state.settings.lock().unwrap().widgets.get(d.id).cloned();
    let pos = match saved.and_then(|s| s.x.zip(s.y)) {
        Some((x, y)) if on_screen(app, x, y) => (x, y),
        _ => default_positions(app).get(d.id).copied().unwrap_or((100, 100)),
    };

    let url = WebviewUrl::App(format!("widget.html?w={}", d.id).into());
    let win = WebviewWindowBuilder::new(app, d.id, url)
        .title(d.title)
        .inner_size(d.w, d.h)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .skip_taskbar(true)
        .always_on_bottom(true)
        .focused(false)
        .visible(false)
        .theme(Some(Theme::Dark))
        .build()?;
    win.set_position(PhysicalPosition::new(pos.0, pos.1))?;
    platform::setup_widget(&win);
    win.show()?;

    let handle = app.clone();
    let id = d.id;
    win.on_window_event(move |e| match e {
        // While the system widget is temporarily expanded its position is not its home.
        WindowEvent::Moved(_) if id == "system" && layout::is_expanded() => {}
        WindowEvent::Moved(p) => {
            let st = handle.state::<AppState>();
            {
                let mut s = st.settings.lock().unwrap();
                let entry = s.widgets.entry(id.into()).or_insert(WidgetState { visible: true, x: None, y: None });
                entry.x = Some(p.x);
                entry.y = Some(p.y);
            }
            if !layout::mouse_down() {
                st.save();
            }
            layout::on_moved(&handle, id, save_settings);
        }
        _ => {}
    });
    Ok(())
}

fn save_settings(app: &AppHandle) {
    app.state::<AppState>().save();
}

fn set_visible(app: &AppHandle, id: &str, visible: bool) {
    let Some(d) = def(id) else { return };
    let st = app.state::<AppState>();
    st.settings.lock().unwrap().widgets.entry(id.into()).or_insert(WidgetState { visible, x: None, y: None }).visible =
        visible;
    st.save();
    if visible {
        let _ = create_widget(app, d);
    } else if let Some(w) = app.get_webview_window(id) {
        let _ = w.destroy();
    }
    refresh_tray(app);
}

fn reset_layout(app: &AppHandle) {
    let positions = default_positions(app);
    let st = app.state::<AppState>();
    for d in DEFS {
        if let (Some(w), Some(&(x, y))) = (app.get_webview_window(d.id), positions.get(d.id)) {
            let _ = w.set_position(PhysicalPosition::new(x, y));
        }
    }
    st.save();
}

// ---------- tray ----------

fn tray_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    let menu = Menu::new(app)?;
    for d in DEFS {
        let visible = s.widgets.get(d.id).map(|w| w.visible).unwrap_or(true);
        menu.append(&CheckMenuItem::with_id(app, format!("toggle:{}", d.id), d.title, true, visible, None::<&str>)?)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "cities", "Города…", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "refresh", "Обновить погоду", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "reset", "Расставить заново", true, None::<&str>)?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    let auto = app.autolaunch().is_enabled().unwrap_or(false);
    menu.append(&CheckMenuItem::with_id(app, "autostart", "Запускать вместе с Windows", true, auto, None::<&str>)?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?)?;
    Ok(menu)
}

fn refresh_tray(app: &AppHandle) {
    if let (Some(tray), Ok(menu)) = (app.tray_by_id("main"), tray_menu(app)) {
        let _ = tray.set_menu(Some(menu));
    }
}

fn handle_menu(app: &AppHandle, id: &str) {
    match id {
        "quit" => app.exit(0),
        "cities" => open_cities_window(app),
        "refresh" => {
            let _ = app.emit("refresh", ());
        }
        "reset" => reset_layout(app),
        "autostart" => {
            let al = app.autolaunch();
            let _ = if al.is_enabled().unwrap_or(false) { al.disable() } else { al.enable() };
            refresh_tray(app);
        }
        _ => {
            if let Some(w) = id.strip_prefix("hide:") {
                set_visible(app, w, false);
            } else if let Some(w) = id.strip_prefix("toggle:") {
                let visible = app.get_webview_window(w).is_some();
                set_visible(app, w, !visible);
            }
        }
    }
}

/// Versions before 1.0 used the bundle id "com.win11widget.app" for their config folder.
fn migrate_legacy_settings(dir: &std::path::Path, path: &std::path::Path) {
    let Some(legacy) = dir.parent().map(|p| p.join("com.win11widget.app").join("settings.json")) else { return };
    if !path.exists() && legacy.exists() {
        let _ = fs::create_dir_all(dir);
        let _ = fs::copy(&legacy, path);
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| open_cities_window(app)))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .invoke_handler(tauri::generate_handler![
            get_settings,
            set_cities,
            system_stats,
            fit_height,
            wallpaper,
            system_expand,
            media::media_state,
            media::media_art,
            media::media_control,
            widget_menu,
            open_cities
        ])
        .on_menu_event(|app, e| handle_menu(app, e.id().as_ref()))
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            let path = dir.join("settings.json");
            migrate_legacy_settings(&dir, &path);
            let mut settings: Settings =
                fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
            for d in DEFS {
                settings.widgets.entry(d.id.into()).or_insert(WidgetState { visible: true, x: None, y: None });
            }
            app.manage(AppState { settings: Mutex::new(settings), path, monitor: OnceLock::new() });

            let handle = app.handle().clone();
            let visible: Vec<&Def> = {
                let s = handle.state::<AppState>();
                let s = s.settings.lock().unwrap();
                DEFS.iter().filter(|d| s.widgets.get(d.id).map(|w| w.visible).unwrap_or(true)).collect()
            };
            for d in visible {
                create_widget(&handle, d)?;
            }

            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().cloned().expect("app icon"))
                .tooltip("Win11Widget")
                .menu(&tray_menu(&handle)?)
                .build(app)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to start Win11Widget")
        .run(|_, e| {
            // Widgets keep running after the cities editor (the only normal window) closes.
            if let RunEvent::ExitRequested { api, code, .. } = e {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
