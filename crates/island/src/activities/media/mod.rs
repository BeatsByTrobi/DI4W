//! Now playing: album art + waveform in the pill, the full player (art, titles, seekable progress, previous /
//! play-pause / next) when expanded, and play/pause + next on hover. Hides a few seconds after pausing.

pub mod meter;
pub mod service;
pub mod waveform;

use crate::model::{Activity, Manager};
use crate::rt::Timer;
use crate::ui::colors::{self, Color};
use crate::ui::el::{Align, Brush, Cursor, El, Len, key};
use crate::ui::geom::{Rect, Size};
use crate::ui::image::Image;
use crate::ui::{self, ALERT_WIDTH, ICON_SIZE, MEDIUM, NORMAL, SEMIBOLD, icons};
use di4w_core::{AppSettings, l};
use service::{Command, Snapshot};
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::time::Duration;

struct State {
    current: Option<Snapshot>,
    art: Option<(String, Rc<Image>)>,
    active: bool,
    hovering: bool,
    hide: Option<Timer>,
    progress: Option<Timer>,
    commands: Option<Sender<Command>>,
}

pub struct MediaActivity {
    manager: Rc<Manager>,
    settings: Rc<RefCell<AppSettings>>,
    any_player: Arc<AtomicBool>,
    st: RefCell<State>,
    me: Weak<MediaActivity>,
}

impl MediaActivity {
    pub fn new(manager: Rc<Manager>, settings: Rc<RefCell<AppSettings>>) -> Rc<MediaActivity> {
        let any = settings.borrow().media_any_player;
        Rc::new_cyclic(|me| MediaActivity {
            manager,
            settings,
            any_player: Arc::new(AtomicBool::new(any)),
            st: RefCell::new(State { current: None, art: None, active: false, hovering: false, hide: None, progress: None, commands: None }),
            me: me.clone(),
        })
    }

    /// Starts reading the media sessions (call once, when music is on in the settings).
    pub fn start(&self) {
        let me = self.me.clone();
        let tx = service::start(self.any_player.clone(), move |snapshot| {
            if let Some(m) = me.upgrade() {
                m.on_updated(snapshot);
            }
        });
        self.st.borrow_mut().commands = Some(tx);
    }

    /// "Show any media player" changed in the settings.
    pub fn settings_changed(&self) {
        let any = self.settings.borrow().media_any_player;
        if self.any_player.swap(any, std::sync::atomic::Ordering::Relaxed) != any {
            self.send(Command::Select);
        }
    }

    fn send(&self, c: Command) {
        if let Some(tx) = &self.st.borrow().commands {
            let _ = tx.send(c);
        }
    }

    fn on_updated(&self, snapshot: Option<Snapshot>) {
        let Some(s) = snapshot else {
            waveform::set_playing(false);
            {
                let mut st = self.st.borrow_mut();
                st.progress = None;
                st.current = None;
                st.active = false;
            }
            self.manager.changed();
            return;
        };
        waveform::set_playing(s.playing);
        {
            let mut st = self.st.borrow_mut();
            match &s.art {
                Some((k, bitmap)) if st.art.as_ref().is_none_or(|(old, _)| old != k) => {
                    st.art = Some((k.clone(), Image::new((**bitmap).clone())));
                }
                None => st.art = None,
                _ => {}
            }
            let playing = s.playing;
            st.current = Some(s);
            if playing {
                st.hide = None;
                st.progress = Some(Timer::every(Duration::from_millis(250), super::redraw));
                st.active = true;
            } else {
                st.progress = None;
                if st.active {
                    let seconds = self.settings.borrow().media_hide_after_pause_seconds.max(1) as u64;
                    let me = self.me.clone();
                    st.hide = Some(Timer::once(Duration::from_secs(seconds), move || {
                        if let Some(m) = me.upgrade() {
                            let playing = m.st.borrow().current.as_ref().is_some_and(|c| c.playing);
                            if !playing {
                                m.st.borrow_mut().active = false;
                                m.manager.changed();
                            }
                        }
                    }));
                }
            }
        }
        self.manager.changed();
    }

    fn accent(&self) -> Color {
        self.st.borrow().current.as_ref().map_or(Color::WHITE, |c| c.accent)
    }

    fn art(&self, size: f64, radius: f64) -> El {
        ui::image(self.st.borrow().art.as_ref().map(|(_, i)| i.id), size, radius)
    }

    fn playing(&self) -> bool {
        self.st.borrow().current.as_ref().is_some_and(|c| c.playing)
    }

    fn action(&self, c: fn() -> Command) -> impl Fn() + 'static {
        let me = self.me.clone();
        move || {
            if let Some(m) = me.upgrade() {
                m.send(c());
            }
        }
    }

    fn open_player(&self) {
        let id = self.st.borrow().current.as_ref().map(|c| c.app_id.clone()).unwrap_or_default();
        if id.to_lowercase().contains("spotify") {
            if crate::interop::focus_process("Spotify") {
                return;
            }
            crate::interop::shell_open("spotify:");
        }
    }

    /// Hovering the pill swaps the waveform for play/pause and next, so you don't have to open the player.
    fn hover_controls(&self) -> El {
        let pp = ui::button(key("media.compact.playpause"), icons::shape(if self.playing() { icons::pause() } else { icons::play() }, 12.0, colors::primary()), self.action(|| Command::TogglePlayPause), None, Some(26.0), Some(26.0), Some(l::t("Play / pause").into()));
        let next = ui::button(key("media.compact.next"), icons::shape(icons::next(), 13.0, colors::primary()), self.action(|| Command::Next), None, Some(26.0), Some(26.0), Some(l::t("Next").into()));
        El::hstack(vec![pp, next]).halign(Align::End)
    }
}

impl Activity for MediaActivity {
    fn name(&self) -> String {
        l::t("Now Playing").to_string()
    }

    fn accessible_detail(&self) -> Option<String> {
        let st = self.st.borrow();
        let c = st.current.as_ref()?;
        Some([c.title.as_str(), c.artist.as_str()].iter().filter(|t| !t.trim().is_empty()).cloned().collect::<Vec<_>>().join(" – "))
    }

    fn priority(&self) -> i32 {
        10
    }

    fn kind(&self) -> String {
        "Media".into()
    }

    fn on_hover(&self, hovering: bool) {
        self.st.borrow_mut().hovering = hovering;
        super::redraw();
    }

    fn is_active(&self) -> bool {
        self.st.borrow().active
    }

    // Album art as the iPhone shows it: a small rounded square in the pill, a round one in the minimal bubble,
    // and a 52 square with corners concentric to the island when expanded.
    fn leading(&self) -> El {
        self.art(24.0, 6.0)
    }

    fn trailing(&self) -> El {
        if self.st.borrow().hovering { self.hover_controls() } else { waveform::view(20.0, 14.0, self.accent()) }.valign(Align::Center)
    }

    fn bubble(&self) -> El {
        self.art(25.0, 12.5)
    }

    /// Mirrors the iOS expanded music island: art + titles, waveform, scrubber, transport controls.
    fn expanded(&self) -> El {
        let (title, artist, position, duration) = {
            let st = self.st.borrow();
            match &st.current {
                Some(c) => (c.title.clone(), c.artist.clone(), c.live_position(), c.duration),
                None => (String::new(), String::new(), 0.0, 0.0),
            }
        };
        let me = self.me.clone();
        let art = self.art(ICON_SIZE, 14.0).cursor(Cursor::Hand).tooltip(l::t("Open Spotify")).on_click(move |_, _| {
            if let Some(m) = me.upgrade() {
                m.open_player();
            }
        });
        let titles = El::vstack(vec![ui::text(title, 16.0, SEMIBOLD, colors::primary()), ui::text(artist, 15.0, NORMAL, colors::grey())]).margin(14.0, 0.0, 10.0, 0.0).valign(Align::Center);
        let wave = waveform::view(24.0, 18.0, self.accent()).valign(Align::Start).margin(0.0, 6.0, 2.0, 0.0);
        let top = El::grid(vec![Len::Auto, Len::Star(1.0), Len::Auto], vec![], vec![(0, 0, art), (0, 1, titles), (0, 2, wave)]);

        // Row 1: elapsed | scrubber | remaining
        let frac = if duration > 0.0 { (position / duration).clamp(0.0, 1.0) } else { 0.0 };
        let me = self.me.clone();
        let bar = El::custom(move |p, r: Rect| {
            let track = Rect::new(r.x, r.y + (r.h - 6.0) / 2.0, r.w, 6.0);
            p.fill_rounded(track, 3.0, Brush::Solid(colors::track()));
            let fill = Rect::new(track.x, track.y, (track.w * frac).max(6.0), 6.0);
            p.fill_rounded(fill, 3.0, Brush::Solid(colors::primary()));
        })
        .h(16.0)
        .valign(Align::Center)
        .cursor(Cursor::Hand)
        .on_click(move |pt, r| {
            if let Some(m) = me.upgrade() {
                let duration = m.st.borrow().current.as_ref().map_or(0.0, |c| c.duration);
                if duration > 0.0 {
                    let f = ((pt.x - r.x) / r.w.max(1.0)).clamp(0.0, 1.0);
                    m.send(Command::Seek(duration * f));
                }
            }
        });
        let elapsed = ui::digits(ui::format_seconds(position), 12.0, MEDIUM, colors::grey());
        let remaining = ui::digits(format!("-{}", ui::format_seconds(duration - position)), 12.0, MEDIUM, colors::grey()).halign(Align::End);
        let progress = El::grid(vec![Len::Px(42.0), Len::Star(1.0), Len::Px(46.0)], vec![], vec![(0, 0, elapsed), (0, 1, bar), (0, 2, remaining)]).margin(2.0, 14.0, 2.0, 0.0);

        // Row 2: ⏮ ⏯ ⏭
        let prev = ui::button(key("media.previous"), icons::shape(icons::previous(), 26.0, colors::primary()), self.action(|| Command::Previous), None, Some(52.0), Some(44.0), Some(l::t("Previous").into()));
        let pp = ui::button(key("media.playpause"), icons::shape(if self.playing() { icons::pause() } else { icons::play() }, 24.0, colors::primary()), self.action(|| Command::TogglePlayPause), None, Some(52.0), Some(44.0), Some(l::t("Play / pause").into()))
            .margin(30.0, 0.0, 30.0, 0.0);
        let next = ui::button(key("media.next"), icons::shape(icons::next(), 26.0, colors::primary()), self.action(|| Command::Next), None, Some(52.0), Some(44.0), Some(l::t("Next").into()));
        let controls = El::hstack(vec![prev, pp, next]).align(Align::Center, Align::Center).margin(0.0, 4.0, 0.0, 0.0);

        El::grid(vec![], vec![Len::Auto, Len::Auto, Len::Star(1.0)], vec![(0, 0, top), (1, 0, progress), (2, 0, controls)]).margin(16.0, 16.0, 16.0, 10.0)
    }

    fn keyline(&self) -> Option<Color> {
        Some(self.accent())
    }

    fn expanded_size(&self) -> Size {
        Size::new(ALERT_WIDTH, 164.0)
    }
}
