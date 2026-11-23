//! Exchange rates: USD→KZT from the mig.kz exchange office, BTC and GRAM in USD from Binance.

use crate::{http, json::Json};

#[derive(Clone, Debug, Default)]
pub struct Ticker {
    pub price: f64,
    /// 24 h change, percent
    pub change: f64,
    /// hourly closes over the last 24 h (for the mini chart)
    pub spark: Vec<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct Rates {
    /// mig.kz: tenge per dollar, the office's selling price
    pub usd: Option<f64>,
    pub btc: Option<Ticker>,
    pub gram: Option<Ticker>,
    /// local time of the last update
    pub updated: Option<windows::Win32::Foundation::SYSTEMTIME>,
}

/// Fetch everything; a source that fails stays None.
pub fn fetch() -> Rates {
    let (usd, binance, k_btc, k_gram) = std::thread::scope(|s| {
        let m = s.spawn(mig);
        let kb = s.spawn(|| klines("BTCUSDT"));
        let kg = s.spawn(|| klines("GRAMUSDT"));
        let b = binance();
        (m.join().ok().flatten(), b, kb.join().ok().flatten(), kg.join().ok().flatten())
    });
    let pick = |sym: &str, spark: Option<Vec<f64>>| {
        binance.as_ref().and_then(|v| v.iter().find(|(s, _)| s == sym)).map(|(_, t)| {
            let mut t = t.clone();
            if let Some(mut sp) = spark {
                sp.push(t.price);
                t.spark = sp;
            }
            t
        })
    };
    Rates { usd, btc: pick("BTCUSDT", k_btc), gram: pick("GRAMUSDT", k_gram), updated: None }
}

/// Hourly closes for the last 24 hours.
fn klines(sym: &str) -> Option<Vec<f64>> {
    let body = http::get("api.binance.com", &format!("/api/v3/klines?symbol={sym}&interval=1h&limit=24"))?;
    let j = Json::parse(&body)?;
    Some(j.arr().iter().filter_map(|k| k.at(4).str().and_then(|s| s.parse().ok())).collect())
}

fn binance() -> Option<Vec<(String, Ticker)>> {
    let body = http::get("api.binance.com", "/api/v3/ticker/24hr?symbols=%5B%22BTCUSDT%22,%22GRAMUSDT%22%5D")?;
    let j = Json::parse(&body)?;
    let num = |v: &Json| v.str().and_then(|s| s.parse::<f64>().ok());
    Some(
        j.arr()
            .iter()
            .filter_map(|t| {
                let sym = t.get("symbol").str()?.to_string();
                Some((
                    sym,
                    Ticker {
                        price: num(t.get("lastPrice"))?,
                        change: num(t.get("priceChangePercent")).unwrap_or(0.0),
                        spark: Vec::new(),
                    },
                ))
            })
            .collect(),
    )
}

/// The rates table on mig.kz: `<td class="buy …">453.3</td><td class="currency">USD</td><td class="sell …">455.9</td>`.
fn mig() -> Option<f64> {
    let html = http::get("mig.kz", "/")?;
    let at = html.find(">USD<")?;
    let cell_value = |s: &str| -> Option<f64> {
        let start = s.find('>')? + 1;
        let end = start + s[start..].find('<')?;
        s[start..end].trim().replace(',', ".").parse().ok()
    };
    let after = &html[at..];
    cell_value(&after[after.find("class=\"sell")?..])
}

/// Russian-style number: thin-space thousands, decimal comma.
pub fn fmt(v: f64, decimals: usize) -> String {
    let s = format!("{:.*}", decimals, v.abs());
    let (int, frac) = s.split_once('.').map_or((s.as_str(), ""), |(a, b)| (a, b));
    let mut out = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push('\u{202f}');
        }
        out.push(c);
    }
    if !frac.is_empty() {
        out.push(',');
        out.push_str(frac);
    }
    if v < 0.0 {
        out.insert(0, '−');
    }
    out
}
