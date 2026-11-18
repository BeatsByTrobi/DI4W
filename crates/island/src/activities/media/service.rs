//! Reads Spotify (or any player) through the Windows media session API, the same source as the volume flyout's
//! media controls. No Spotify account or API key needed.
//!
//! The media session objects live on a background thread (WinRT calls block there instead of on the UI thread);
//! the UI thread gets a [`Snapshot`] each time something changes and sends commands (play/pause, seek) back.

use crate::ui::colors::{self, Color};
use crate::ui::image::{self, Bitmap, Decode};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as SessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};

/// What's playing, as of `taken_at`.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub title: String,
    pub artist: String,
    /// Album art (cropped square, about 240 px), with a key that changes with the song.
    pub art: Option<(String, Arc<Bitmap>)>,
    pub accent: Color,
    pub playing: bool,
    pub position: f64,
    pub duration: f64,
    pub taken_at: Instant,
    pub app_id: String,
}

impl Snapshot {
    /// Position extrapolated to "now" while playing (apps only report it occasionally).
    pub fn live_position(&self) -> f64 {
        let mut p = if self.playing { self.position + self.taken_at.elapsed().as_secs_f64() } else { self.position };
        if self.duration > 0.0 && p > self.duration {
            p = self.duration;
        }
        p.max(0.0)
    }
}

pub enum Command {
    Select,
    Refresh,
    TogglePlayPause,
    Next,
    Previous,
    /// Seconds.
    Seek(f64),
}

/// Starts the session watcher; `publish` runs on the UI thread with every new state (`None`: nothing playing).
pub fn start(any_player: Arc<AtomicBool>, publish: impl Fn(Option<Snapshot>) + 'static) -> Sender<Command> {
    let (tx, rx) = channel();
    let id = crate::rt::Handle::new(publish);
    let sender = tx.clone();
    crate::rt::background(move || run(rx, sender, any_player, id));
    tx
}

struct Watching {
    session: Session,
    tokens: [i64; 3],
}

fn run(rx: Receiver<Command>, tx: Sender<Command>, any_player: Arc<AtomicBool>, publish: crate::rt::Handle<Option<Snapshot>>) {
    let manager = match SessionManager::RequestAsync().and_then(|op| op.join()) {
        Ok(m) => m,
        Err(e) => {
            di4w_core::log::error_with("Media session manager unavailable", e);
            return;
        }
    };
    for event in 0..2 {
        let tx = tx.clone();
        let handler = move || {
            let _ = tx.send(Command::Select);
        };
        let _ = if event == 0 {
            manager.SessionsChanged(&TypedEventHandler::new(move |_, _| {
                handler();
                Ok(())
            }))
        } else {
            manager.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
                handler();
                Ok(())
            }))
        };
    }

    let mut watching: Option<Watching> = None;
    let mut art: Option<(String, Arc<Bitmap>, Color)> = None;
    let _ = tx.send(Command::Select);
    while let Ok(command) = rx.recv() {
        match command {
            Command::Select => {
                let pick = select(&manager, any_player.load(Ordering::Relaxed));
                let same = match (&pick, &watching) {
                    (Some(p), Some(w)) => p.SourceAppUserModelId().ok() == w.session.SourceAppUserModelId().ok(),
                    (None, None) => true,
                    _ => false,
                };
                if !same {
                    if let Some(w) = watching.take() {
                        let _ = w.session.RemoveMediaPropertiesChanged(w.tokens[0]);
                        let _ = w.session.RemovePlaybackInfoChanged(w.tokens[1]);
                        let _ = w.session.RemoveTimelinePropertiesChanged(w.tokens[2]);
                    }
                    if let Some(session) = pick {
                        di4w_core::log::info(format!("Media session: {}", session.SourceAppUserModelId().map(|s| s.to_string()).unwrap_or_default()));
                        let changed = |tx: Sender<Command>| move || {
                            let _ = tx.send(Command::Refresh);
                        };
                        let (a, b, c) = (changed(tx.clone()), changed(tx.clone()), changed(tx.clone()));
                        let tokens = [
                            session.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                                a();
                                Ok(())
                            })).unwrap_or_default(),
                            session.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                                b();
                                Ok(())
                            })).unwrap_or_default(),
                            session.TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
                                c();
                                Ok(())
                            })).unwrap_or_default(),
                        ];
                        watching = Some(Watching { session, tokens });
                    }
                }
                let _ = tx.send(Command::Refresh);
            }
            Command::Refresh => {
                // Several events usually fire together (track change); coalesce them.
                std::thread::sleep(Duration::from_millis(60));
                let mut reselect = false;
                while let Ok(next) = rx.try_recv() {
                    match next {
                        Command::Refresh => {}
                        Command::Select => reselect = true,
                        other => perform(watching.as_ref().map(|w| &w.session), other),
                    }
                }
                if reselect {
                    let _ = tx.send(Command::Select);
                }
                let snapshot = watching.as_ref().and_then(|w| read(&w.session, &mut art));
                publish.send(snapshot);
            }
            other => perform(watching.as_ref().map(|w| &w.session), other),
        }
    }
}

fn select(manager: &SessionManager, any_player: bool) -> Option<Session> {
    let sessions = match manager.GetSessions() {
        Ok(s) => s,
        Err(e) => {
            di4w_core::log::error_with("Could not enumerate media sessions", e);
            return None;
        }
    };
    let spotify = (&sessions).into_iter().find(|s| s.SourceAppUserModelId().is_ok_and(|id| id.to_string().to_lowercase().contains("spotify")));
    spotify.or_else(|| if any_player { manager.GetCurrentSession().ok() } else { None })
}

fn perform(session: Option<&Session>, command: Command) {
    let Some(s) = session else { return };
    let result = match command {
        Command::TogglePlayPause => s.TryTogglePlayPauseAsync().and_then(|o| o.join()),
        Command::Next => s.TrySkipNextAsync().and_then(|o| o.join()),
        Command::Previous => s.TrySkipPreviousAsync().and_then(|o| o.join()),
        Command::Seek(secs) => s.TryChangePlaybackPositionAsync((secs * 10_000_000.0) as i64).and_then(|o| o.join()),
        _ => Ok(true),
    };
    if let Err(e) = result {
        di4w_core::log::error_with("Media command failed", e);
    }
}

fn read(session: &Session, art: &mut Option<(String, Arc<Bitmap>, Color)>) -> Option<Snapshot> {
    let result = (|| -> windows::core::Result<Option<Snapshot>> {
        let props = session.TryGetMediaPropertiesAsync()?.join()?;
        let title = props.Title()?.to_string();
        if title.trim().is_empty() {
            return Ok(None);
        }
        let artist = props.Artist().map(|a| a.to_string()).unwrap_or_default();
        let key = format!("{}|{}|{}", title, artist, props.AlbumTitle().map(|a| a.to_string()).unwrap_or_default());
        if art.as_ref().is_none_or(|(k, _, _)| *k != key) {
            *art = props.Thumbnail().ok().and_then(|t| load_art(&t)).map(|b| {
                let accent = dominant_color(&b);
                (key.clone(), Arc::new(b), accent)
            });
        }
        let playing = session.GetPlaybackInfo().and_then(|p| p.PlaybackStatus()).is_ok_and(|s| s == Status::Playing);
        let timeline = session.GetTimelineProperties()?;
        let duration = (timeline.EndTime()?.Duration - timeline.StartTime()?.Duration) as f64 / 1e7;
        let position = timeline.Position()?.Duration as f64 / 1e7;
        // When the app last reported the position; "never" (year < 2000) means now.
        let updated = timeline.LastUpdatedTime().map(|d| d.UniversalTime).unwrap_or(0);
        let now = filetime_now();
        let age = (now - updated) as f64 / 1e7;
        let taken_at = if updated < 125_911_584_000_000_000 || !(0.0..86_400.0).contains(&age) {
            Instant::now()
        } else {
            Instant::now().checked_sub(Duration::from_secs_f64(age)).unwrap_or_else(Instant::now)
        };
        Ok(Some(Snapshot {
            title,
            artist,
            art: art.as_ref().map(|(k, b, _)| (k.clone(), b.clone())),
            accent: art.as_ref().map_or_else(colors::primary, |(_, _, c)| *c),
            playing,
            position,
            duration,
            taken_at,
            app_id: session.SourceAppUserModelId().map(|s| s.to_string()).unwrap_or_default(),
        }))
    })();
    result.unwrap_or_else(|e| {
        di4w_core::log::error_with("Media refresh failed", e);
        None
    })
}

/// Now, in 100 ns ticks since 1601 (WinRT's DateTime).
fn filetime_now() -> i64 {
    let ft = unsafe { windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime() };
    ((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64
}

/// Reads a WinRT stream reference completely (album art, app logos).
pub fn read_stream(reference: &IRandomAccessStreamReference) -> Option<Vec<u8>> {
    let stream = reference.OpenReadAsync().ok()?.join().ok()?;
    let size = stream.Size().ok()? as u32;
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    reader.LoadAsync(size).ok()?.join().ok()?;
    let mut bytes = vec![0u8; size as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    Some(bytes)
}

fn load_art(reference: &IRandomAccessStreamReference) -> Option<Bitmap> {
    let bytes = read_stream(reference)?;
    // Spotify's thumbnails can include a transparent/letterboxed border; crop to the centered square.
    match image::decode(&bytes, Decode { width: Some(240), height: None, square: true }) {
        Ok(b) => Some(b),
        Err(e) => {
            di4w_core::log::error_with("Could not load album art", e);
            None
        }
    }
}

/// Average colour weighted towards saturated pixels, lifted so it stays visible on black (or darkened so it reads
/// on the light island).
pub fn dominant_color(art: &Bitmap) -> Color {
    let small = image::resample(art, 16, 16);
    let (mut r, mut g, mut b, mut total) = (0.0, 0.0, 0.0, 0.0);
    for px in small.pixels.chunks_exact(4) {
        let (pb, pg, pr) = (px[0] as f64 / 255.0, px[1] as f64 / 255.0, px[2] as f64 / 255.0);
        let max = pr.max(pg).max(pb);
        let min = pr.min(pg).min(pb);
        let sat = if max == 0.0 { 0.0 } else { (max - min) / max };
        let weight = sat * sat * max + 0.02;
        r += pr * weight;
        g += pg * weight;
        b += pb * weight;
        total += weight;
    }
    if total <= 0.0 {
        return colors::primary();
    }
    r /= total;
    g /= total;
    b /= total;
    let mut lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let byte = |v: f64| (v * 255.0) as u8;
    if colors::light() {
        // On the light island: darken light colours until they read on near-white (keeping the hue).
        if lum > 0.4 {
            let scale = 0.4 / lum;
            r *= scale;
            g *= scale;
            b *= scale;
        }
        return Color::rgb(byte(r), byte(g), byte(b));
    }
    // Lift dark colours towards a readable brightness.
    if lum < 0.55 {
        let max = r.max(g).max(b);
        let scale = if max > 0.0 { (1.0 / max).min(0.6 / lum.max(0.05)) } else { 1.0 };
        r = (r * scale).min(1.0);
        g = (g * scale).min(1.0);
        b = (b * scale).min(1.0);
        lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        if lum < 0.45 {
            let t = (0.45 - lum) / (1.0 - lum);
            r += (1.0 - r) * t;
            g += (1.0 - g) * t;
            b += (1.0 - b) * t;
        }
    }
    Color::rgb(byte(r), byte(g), byte(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_art_gets_a_readable_accent() {
        let navy = Bitmap { w: 2, h: 2, pixels: [40u8, 10, 10, 255].repeat(4) };
        let c = dominant_color(&navy);
        let lum = 0.2126 * c.r as f64 + 0.7152 * c.g as f64 + 0.0722 * c.b as f64;
        assert!(lum / 255.0 >= 0.44, "{c:?}");
        assert!(c.b > c.r, "keeps the hue: {c:?}");
    }

    #[test]
    fn live_position_runs_while_playing() {
        let s = Snapshot { title: "x".into(), artist: String::new(), art: None, accent: Color::WHITE, playing: true, position: 10.0, duration: 200.0, taken_at: Instant::now() - Duration::from_secs(5), app_id: String::new() };
        assert!((s.live_position() - 15.0).abs() < 0.5);
        let paused = Snapshot { playing: false, ..s };
        assert_eq!(paused.live_position(), 10.0);
    }
}
