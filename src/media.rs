//! "Now playing" via Windows' Global System Media Transport Controls — the same source
//! the volume flyout uses. Works with any player that reports to Windows (Spotify,
//! browsers, Yandex Music, …); no third-party software involved.
//!
//! Runs on its own thread (WinRT calls block); results are posted to the UI thread.

use std::{
    sync::{mpsc, Mutex},
    time::Duration,
};

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

use crate::image::{self, Image};

/// FILETIME epoch (1601) to Unix epoch, in 100 ns ticks.
const EPOCH_DIFF: i64 = 116_444_736_000_000_000;
const POLL: Duration = Duration::from_millis(1500);
/// Cover art is decoded once to this square size.
const ART_PX: u32 = 256;

#[derive(Clone, Default, Debug)]
pub struct MediaState {
    pub active: bool,
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Friendly player name, e.g. "Spotify".
    pub source: String,
    /// Changes whenever the track (and so the cover) changes.
    pub art_key: String,
    /// Seconds.
    pub position: f64,
    pub duration: f64,
    /// Unix ms when `position` was sampled; the UI extrapolates while playing.
    pub updated_at: f64,
    pub can_prev: bool,
    pub can_next: bool,
}

pub struct Shared {
    pub state: MediaState,
    pub art: Option<Image>,
    /// Bumped whenever `art` changes.
    pub art_gen: u32,
}

pub static SHARED: Mutex<Shared> = Mutex::new(Shared { state: empty(), art: None, art_gen: 0 });

const fn empty() -> MediaState {
    MediaState {
        active: false,
        playing: false,
        title: String::new(),
        artist: String::new(),
        album: String::new(),
        source: String::new(),
        art_key: String::new(),
        position: 0.0,
        duration: 0.0,
        updated_at: 0.0,
        can_prev: false,
        can_next: false,
    }
}

#[derive(Clone, Copy)]
pub enum Cmd {
    Prev,
    Toggle,
    Next,
}

/// Starts the media thread; `notify` is called after every update (from that thread).
pub fn spawn(notify: impl Fn() + Send + 'static) -> mpsc::Sender<Cmd> {
    let (tx, rx) = mpsc::channel::<Cmd>();
    std::thread::spawn(move || {
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
        let mut manager: Option<Manager> = None;
        let mut art_key = String::new();
        // Players often publish the cover a moment after the title: retry a few polls.
        let mut art_retries = 0u8;
        loop {
            if manager.is_none() {
                manager = Manager::RequestAsync().and_then(|op| op.join()).ok();
            }
            let session = manager.as_ref().and_then(|m| m.GetCurrentSession().ok());
            let st = session.as_ref().and_then(|s| read_state(s).ok()).unwrap_or_default();
            let missing = st.active && SHARED.lock().unwrap().art.is_none();
            let new_art = if st.art_key != art_key || (missing && art_retries > 0) {
                if st.art_key != art_key {
                    art_retries = 4;
                }
                art_key = st.art_key.clone();
                art_retries = art_retries.saturating_sub(1);
                let art = if st.active { session.as_ref().and_then(|s| read_art(s).ok().flatten()) } else { None };
                if art.is_some() {
                    art_retries = 0;
                }
                Some(art)
            } else {
                None
            };
            {
                let mut g = SHARED.lock().unwrap();
                g.state = st;
                if let Some(a) = new_art {
                    g.art = a;
                    g.art_gen = g.art_gen.wrapping_add(1);
                }
            }
            notify();

            match rx.recv_timeout(POLL) {
                Ok(cmd) => {
                    if let Some(s) = &session {
                        let op = match cmd {
                            Cmd::Toggle => s.TryTogglePlayPauseAsync(),
                            Cmd::Next => s.TrySkipNextAsync(),
                            Cmd::Prev => s.TrySkipPreviousAsync(),
                        };
                        let _ = op.and_then(|o| o.join());
                    }
                    std::thread::sleep(Duration::from_millis(250));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    });
    tx
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

fn read_state(s: &Session) -> Result<MediaState> {
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

fn read_art(s: &Session) -> Result<Option<Image>> {
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
    Ok(image::decode_cover(image::Source::Bytes(&bytes), ART_PX, ART_PX))
}
