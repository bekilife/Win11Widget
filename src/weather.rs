//! Open-Meteo forecast and geocoding (free, no API key) — same requests as before.

use crate::{http, json::Json, settings::City};

#[derive(Clone, Debug)]
pub struct Forecast {
    pub utc_offset: i64,
    pub temp: f64,
    pub code: i32,
    pub is_day: bool,
    /// "2026-10-18T03:00"
    pub time: String,
    pub hourly: Vec<Hour>,
    pub max: f64,
    pub min: f64,
}

#[derive(Clone, Debug)]
pub struct Hour {
    pub time: String,
    pub temp: f64,
    pub code: i32,
    pub is_day: bool,
}

pub fn fetch(c: &City) -> Option<Forecast> {
    let path = format!(
        "/v1/forecast?latitude={}&longitude={}&current=temperature_2m,apparent_temperature,weather_code,is_day\
         &hourly=temperature_2m,weather_code,is_day&daily=temperature_2m_max,temperature_2m_min&timezone=auto&forecast_days=2",
        c.lat, c.lon
    );
    let body = http::get("api.open-meteo.com", &path)?;
    let j = Json::parse(&body)?;
    let cur = j.get("current");
    let h = j.get("hourly");
    let times = h.get("time").arr();
    let hourly = times
        .iter()
        .enumerate()
        .map(|(i, t)| Hour {
            time: t.str().unwrap_or("").to_string(),
            temp: h.get("temperature_2m").at(i).num().unwrap_or(0.0),
            code: h.get("weather_code").at(i).num().unwrap_or(0.0) as i32,
            is_day: h.get("is_day").at(i).num().unwrap_or(1.0) != 0.0,
        })
        .collect();
    Some(Forecast {
        utc_offset: j.get("utc_offset_seconds").num().unwrap_or(0.0) as i64,
        temp: cur.get("temperature_2m").num()?,
        code: cur.get("weather_code").num().unwrap_or(0.0) as i32,
        is_day: cur.get("is_day").num().unwrap_or(1.0) != 0.0,
        time: cur.get("time").str().unwrap_or("").to_string(),
        hourly,
        max: j.get("daily").get("temperature_2m_max").at(0).num().unwrap_or(0.0),
        min: j.get("daily").get("temperature_2m_min").at(0).num().unwrap_or(0.0),
    })
}

/// City search in any language; results named in Russian.
pub fn search(text: &str) -> Option<Vec<City>> {
    let path = format!("/v1/search?name={}&count=8&language=ru&format=json", http::encode(text));
    let j = Json::parse(&http::get("geocoding-api.open-meteo.com", &path)?)?;
    Some(
        j.get("results")
            .arr()
            .iter()
            .filter_map(|r| {
                Some(City {
                    name: r.get("name").str()?.to_string(),
                    admin: r.get("admin1").str().unwrap_or("").to_string(),
                    country: r.get("country").str().unwrap_or("").to_string(),
                    lat: r.get("latitude").num()?,
                    lon: r.get("longitude").num()?,
                })
            })
            .collect(),
    )
}

pub fn text(code: i32) -> &'static str {
    match code {
        0 => "Ясно",
        1 => "Преимущественно ясно",
        2 => "Переменная облачность",
        3 => "Пасмурно",
        45 => "Туман",
        48 => "Изморозь",
        51 => "Слабая морось",
        53 => "Морось",
        55 => "Сильная морось",
        56 | 57 => "Ледяная морось",
        61 => "Небольшой дождь",
        63 => "Дождь",
        65 => "Сильный дождь",
        66 | 67 => "Ледяной дождь",
        71 => "Небольшой снег",
        73 => "Снег",
        75 => "Сильный снег",
        77 => "Снежная крупа",
        80 | 81 => "Ливень",
        82 => "Сильный ливень",
        85 => "Снегопад",
        86 => "Сильный снегопад",
        95 => "Гроза",
        96 | 99 => "Гроза с градом",
        _ => "",
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    ClearDay,
    ClearNight,
    PartlyDay,
    PartlyNight,
    Cloudy,
    Fog,
    Drizzle,
    Rain,
    Snow,
    Sleet,
    Thunder,
}

pub fn kind(code: i32, day: bool) -> Kind {
    match code {
        0 | 1 => {
            if day {
                Kind::ClearDay
            } else {
                Kind::ClearNight
            }
        }
        2 => {
            if day {
                Kind::PartlyDay
            } else {
                Kind::PartlyNight
            }
        }
        3 => Kind::Cloudy,
        45 | 48 => Kind::Fog,
        56 | 57 | 66 | 67 => Kind::Sleet,
        51..=55 => Kind::Drizzle,
        61..=65 | 80..=82 => Kind::Rain,
        71..=77 | 85 | 86 => Kind::Snow,
        95.. => Kind::Thunder,
        _ => Kind::Cloudy,
    }
}

/// Current wall-clock time in the city ("HH:MM"), from Open-Meteo's UTC offset.
pub fn city_time(f: &Forecast) -> String {
    let now =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let t = (now + f.utc_offset).rem_euclid(86400);
    format!("{:02}:{:02}", t / 3600, (t % 3600) / 60)
}

pub fn deg(v: f64) -> String {
    // JS Math.round: halves go up.
    format!("{}°", (v + 0.5).floor() as i64)
}
