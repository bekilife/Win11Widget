//! settings.json — same file and format as before
//! (%APPDATA%\com.win11widget.desktop\settings.json), so existing settings carry over.

use std::{fs, path::PathBuf};

use crate::json::Json;

#[derive(Clone, Debug, PartialEq)]
pub struct City {
    pub name: String,
    pub admin: String,
    pub country: String,
    pub lat: f64,
    pub lon: f64,
}

impl City {
    pub fn key(&self) -> String {
        format!("{},{}", self.lat, self.lon)
    }
    pub fn place(&self) -> String {
        [self.admin.as_str(), self.country.as_str()]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(", ")
    }
    fn from_json(j: &Json) -> Option<City> {
        Some(City {
            name: j.get("name").str()?.to_string(),
            admin: j.get("admin").str().unwrap_or("").to_string(),
            country: j.get("country").str().unwrap_or("").to_string(),
            lat: j.get("lat").num()?,
            lon: j.get("lon").num()?,
        })
    }
    fn to_json(&self) -> Json {
        Json::Obj(vec![
            ("name".into(), Json::Str(self.name.clone())),
            ("admin".into(), Json::Str(self.admin.clone())),
            ("country".into(), Json::Str(self.country.clone())),
            ("lat".into(), Json::Num(self.lat)),
            ("lon".into(), Json::Num(self.lon)),
        ])
    }
}

#[derive(Clone, Debug)]
pub struct WidgetState {
    pub visible: bool,
    pub x: Option<i32>,
    pub y: Option<i32>,
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub cities: Vec<City>,
    /// Keeps the order of the file (and of first appearance).
    pub widgets: Vec<(String, WidgetState)>,
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
            widgets: Vec::new(),
        }
    }
}

impl Settings {
    pub fn widget(&mut self, id: &str) -> &mut WidgetState {
        if let Some(i) = self.widgets.iter().position(|(k, _)| k == id) {
            return &mut self.widgets[i].1;
        }
        self.widgets.push((id.to_string(), WidgetState { visible: true, x: None, y: None }));
        &mut self.widgets.last_mut().unwrap().1
    }
    pub fn get(&self, id: &str) -> Option<&WidgetState> {
        self.widgets.iter().find(|(k, _)| k == id).map(|(_, w)| w)
    }
    pub fn visible(&self, id: &str) -> bool {
        self.get(id).map(|w| w.visible).unwrap_or(true)
    }

    fn from_json(j: &Json) -> Option<Settings> {
        let cities = j.get("cities").arr().iter().filter_map(City::from_json).collect();
        let widgets = j
            .get("widgets")
            .entries()
            .iter()
            .map(|(k, v)| {
                let pos = |n: &str| v.get(n).num().map(|f| f as i32);
                (k.clone(), WidgetState { visible: v.get("visible").bool().unwrap_or(true), x: pos("x"), y: pos("y") })
            })
            .collect();
        matches!(j, Json::Obj(_)).then_some(Settings { cities, widgets })
    }

    fn to_json(&self) -> Json {
        let opt = |v: Option<i32>| v.map_or(Json::Null, |n| Json::Num(n as f64));
        Json::Obj(vec![
            ("cities".into(), Json::Arr(self.cities.iter().map(City::to_json).collect())),
            (
                "widgets".into(),
                Json::Obj(
                    self.widgets
                        .iter()
                        .map(|(k, w)| {
                            (
                                k.clone(),
                                Json::Obj(vec![
                                    ("visible".into(), Json::Bool(w.visible)),
                                    ("x".into(), opt(w.x)),
                                    ("y".into(), opt(w.y)),
                                ]),
                            )
                        })
                        .collect(),
                ),
            ),
        ])
    }
}

pub fn path() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("com.win11widget.desktop").join("settings.json")
}

pub fn load() -> Settings {
    let path = path();
    // Versions before 1.0 used "com.win11widget.app" for their config folder.
    if !path.exists() {
        if let Some(legacy) =
            path.parent().and_then(|d| d.parent()).map(|p| p.join("com.win11widget.app").join("settings.json"))
        {
            if legacy.exists() {
                if let Some(dir) = path.parent() {
                    let _ = fs::create_dir_all(dir);
                }
                let _ = fs::copy(&legacy, &path);
            }
        }
    }
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| Json::parse(&s))
        .and_then(|j| Settings::from_json(&j))
        .unwrap_or_default()
}

pub fn save(s: &Settings) {
    let path = path();
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(&path, s.to_json().pretty());
}
