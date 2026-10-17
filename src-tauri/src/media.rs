//! "Now playing" via Windows' Global System Media Transport Controls — the same source
//! the volume flyout uses. Works with any player that reports to Windows (Spotify,
//! browsers, Yandex Music, …); no third-party software involved.

use std::sync::Mutex;

use serde::Serialize;
use windows::{
    core::Result,
    Media::Control::{
        GlobalSystemMediaTransportControlsSession as Session,
        GlobalSystemMediaTransportControlsSessionManager as Manager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
    },
    Storage::Streams::{DataReader, IRandomAccessStreamReference},
    Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED},
};

/// FILETIME epoch (1601) to Unix epoch, in 100 ns ticks.
const EPOCH_DIFF: i64 = 116_444_736_000_000_000;

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaState {
    active: bool,
    playing: bool,
    title: String,
    artist: String,
    album: String,
    /// Friendly player name, e.g. "Spotify".
    source: String,
    /// Changes whenever the track (and so the cover) changes.
    art_key: String,
    /// Seconds.
    position: f64,
    duration: f64,
    /// Unix ms when `position` was sampled; the UI extrapolates while playing.
    updated_at: f64,
    can_prev: bool,
    can_next: bool,
}

static MANAGER: Mutex<Option<Manager>> = Mutex::new(None);

fn manager() -> Result<Manager> {
    // Called from Tauri's worker threads; WinRT needs an apartment (MTA is fine).
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
    let mut guard = MANAGER.lock().unwrap();
    if let Some(m) = guard.as_ref() {
        return Ok(m.clone());
    }
    let m = Manager::RequestAsync()?.join()?;
    *guard = Some(m.clone());
    Ok(m)
}

fn session() -> Option<Session> {
    manager().ok()?.GetCurrentSession().ok()
}

/// "Spotify.exe" -> "Spotify", "Chrome" -> "Chrome", "308046B0AF4A39CB" (Firefox) -> "Firefox", …
fn friendly_source(aumid: &str) -> String {
    let lower = aumid.to_lowercase();
    let known = [
        ("spotify", "Spotify"),
        ("chrome", "Chrome"),
        ("msedge", "Edge"),
        ("firefox", "Firefox"),
        ("308046b0af4a39cb", "Firefox"),
        ("yandex", "Яндекс"),
        ("opera", "Opera"),
        ("brave", "Brave"),
        ("telegram", "Telegram"),
        ("vlc", "VLC"),
        ("zunemusic", "Медиаплеер"),
        ("microsoft.media.player", "Медиаплеер"),
        ("music.ui", "Медиаплеер"),
        ("applemusic", "Apple Music"),
        ("aimp", "AIMP"),
        ("foobar", "foobar2000"),
    ];
    if let Some((_, name)) = known.iter().find(|(k, _)| lower.contains(k)) {
        return name.to_string();
    }
    let base = aumid.rsplit(['\\', '!']).next().unwrap_or(aumid);
    base.trim_end_matches(".exe").trim_end_matches(".EXE").to_string()
}

fn read_state() -> Result<MediaState> {
    let Some(s) = session() else { return Ok(MediaState::default()) };
    let props = s.TryGetMediaPropertiesAsync()?.join()?;
    let info = s.GetPlaybackInfo()?;
    let controls = info.Controls()?;
    let timeline = s.GetTimelineProperties()?;

    let title = props.Title()?.to_string();
    let artist = props.Artist()?.to_string();
    let album = props.AlbumTitle()?.to_string();
    let duration = (timeline.EndTime()?.Duration - timeline.StartTime()?.Duration) as f64 / 1e7;
    let updated = timeline.LastUpdatedTime()?.UniversalTime;

    Ok(MediaState {
        active: !title.is_empty() || !artist.is_empty(),
        playing: info.PlaybackStatus()? == Status::Playing,
        art_key: format!("{title}\u{1f}{artist}\u{1f}{album}"),
        title,
        artist,
        album,
        source: friendly_source(&s.SourceAppUserModelId()?.to_string()),
        position: timeline.Position()?.Duration as f64 / 1e7,
        duration: duration.max(0.0),
        updated_at: if updated > 0 { (updated - EPOCH_DIFF) as f64 / 1e4 } else { 0.0 },
        can_prev: controls.IsPreviousEnabled()?,
        can_next: controls.IsNextEnabled()?,
    })
}

fn read_art() -> Result<Option<String>> {
    use base64::Engine;
    let Some(s) = session() else { return Ok(None) };
    let thumb: IRandomAccessStreamReference = match s.TryGetMediaPropertiesAsync()?.join()?.Thumbnail() {
        Ok(t) => t,
        Err(_) => return Ok(None),
    };
    let stream = thumb.OpenReadAsync()?.join()?;
    let size = stream.Size()? as u32;
    if size == 0 {
        return Ok(None);
    }
    let reader = DataReader::CreateDataReader(&stream)?;
    reader.LoadAsync(size)?.join()?;
    let mut bytes = vec![0u8; size as usize];
    reader.ReadBytes(&mut bytes)?;
    let mime = if bytes.starts_with(&[0x89, b'P']) { "image/png" } else { "image/jpeg" };
    Ok(Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))))
}

// ---------- commands (async: run on worker threads, never block the UI thread) ----------

#[tauri::command]
pub async fn media_state() -> MediaState {
    read_state().unwrap_or_default()
}

#[tauri::command]
pub async fn media_art() -> Option<String> {
    read_art().ok().flatten()
}

#[tauri::command]
pub async fn media_control(action: String) -> bool {
    let Some(s) = session() else { return false };
    let op = match action.as_str() {
        "toggle" => s.TryTogglePlayPauseAsync(),
        "next" => s.TrySkipNextAsync(),
        "prev" => s.TrySkipPreviousAsync(),
        _ => return false,
    };
    op.and_then(|o| o.join()).unwrap_or(false)
}
