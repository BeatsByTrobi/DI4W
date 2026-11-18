//! The Island API: scripts and apps on this PC show their own pop-ups and progress in the island.
//!
//! ```text
//! POST /alert     {"title":"Backup done","message":"312 files","icon":"success","color":"green","seconds":5,"url":"https://…"}
//! GET  /alert/<id>  → {"id":…,"state":"open"|"clicked"|"dismissed","button":…}
//! POST /progress  {"id":"backup","title":"Backing up","value":0.4,"detail":"120 of 312 files"}
//! POST /progress  {"id":"backup","done":true,"message":"Backup done"}
//! ```
//!
//! Requests from web pages are refused by the server (see [`server`]). `tools/island.ps1` wraps it.

pub mod server;

use crate::addons::activity::AddonActivity;
use crate::model::{Alert, Manager};
use crate::rt::Timer;
use crate::ui::colors::{self, Color};
use crate::ui::el::{Align, El, key};
use crate::ui::{self, ACTION_ALERT_HEIGHT, ALERT_HEIGHT, ALERT_WIDTH, ICON_SIZE, NORMAL, SEMIBOLD, icons};
use serde::Serialize;
use serde_json::Value;
use server::{Request, Response};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};

const MAX_PROGRESS: usize = 5;
const BAR_WIDTH: f64 = ALERT_WIDTH - 2.0 * 18.0;
const MAX_ALERTS_PER_MINUTE: usize = 30;
const PROGRESS_EXPIRY: Duration = Duration::from_secs(15 * 60);

/// What happened to an alert: "open", "clicked" (with the button's id, or none for the alert itself) or
/// "dismissed". Scripts poll GET /alert/<id>; kept for 15 minutes.
struct AlertResult {
    state: RefCell<String>,
    button: RefCell<Option<String>>,
    created: Instant,
}

struct ProgressItem {
    activity: Rc<AddonActivity>,
    view: Rc<RefCell<ProgressView>>,
    updated: Instant,
}

struct ProgressView {
    title: String,
    detail: String,
    value: Option<f64>,
    color: Color,
}

pub struct IslandApi {
    manager: Rc<Manager>,
    progress: RefCell<HashMap<String, ProgressItem>>,
    results: RefCell<HashMap<String, Rc<AlertResult>>>,
    recent_alerts: RefCell<VecDeque<Instant>>,
    _expire: Cell<Option<Timer>>,
}

thread_local! {
    static API: RefCell<Option<Rc<IslandApi>>> = const { RefCell::new(None) };
}

/// The icons scripts can ask for.
pub const ICON_NAMES: [(&str, &str); 16] = [
    ("info", "\u{E946}"), ("success", "\u{E73E}"), ("warning", "\u{E7BA}"), ("error", "\u{E711}"),
    ("download", "\u{E896}"), ("upload", "\u{E898}"), ("music", "\u{E8D6}"), ("mail", "\u{E715}"),
    ("timer", "\u{E916}"), ("bell", "\u{EA8F}"), ("code", "\u{E943}"), ("home", "\u{E80F}"),
    ("star", "\u{E734}"), ("heart", "\u{EB51}"), ("game", "\u{E7FC}"), ("sync", "\u{E895}"),
];

fn named_color(name: &str) -> Option<Color> {
    Some(match name.to_lowercase().as_str() {
        "blue" => colors::blue(),
        "green" => colors::green(),
        "orange" => colors::orange(),
        "red" => colors::red(),
        "purple" => colors::purple(),
        "teal" => colors::teal(),
        "yellow" => colors::amber(),
        "indigo" => colors::indigo(),
        "gray" | "grey" => colors::button_bg(),
        _ => return None,
    })
}

impl IslandApi {
    /// Creates the API and starts its server on 127.0.0.1:`port`; the server lives as long as the returned handle.
    pub fn start(manager: Rc<Manager>, port: u16) -> Option<(Rc<IslandApi>, server::Server)> {
        let api = Rc::new(IslandApi {
            manager,
            progress: RefCell::new(HashMap::new()),
            results: RefCell::new(HashMap::new()),
            recent_alerts: RefCell::new(VecDeque::new()),
            _expire: Cell::new(None),
        });
        let weak = Rc::downgrade(&api);
        api._expire.set(Some(Timer::every(Duration::from_secs(60), move || {
            if let Some(a) = weak.upgrade() {
                a.expire_stale();
            }
        })));
        API.with(|a| *a.borrow_mut() = Some(api.clone()));
        let server = server::Server::start(port, |request| {
            if request.path == "/" {
                return Some(Response::ok());
            }
            handle_on_ui(request)
        })?;
        Some((api, server))
    }

    /// Answers a request (UI thread); `None` means "not an API path".
    pub fn handle(&self, request: &Request) -> Option<Response> {
        let path = request.path.trim_end_matches('/').to_lowercase();
        match (request.method.as_str(), path.as_str()) {
            ("GET", "/api") | ("GET", "") => {
                #[derive(Serialize)]
                struct Info {
                    app: &'static str,
                    version: String,
                    endpoints: [&'static str; 3],
                }
                Some(Response::json(200, &Info {
                    app: "Dynamic Island for Windows",
                    version: di4w_core::updates::current_version().to_string(),
                    endpoints: ["POST /alert", "GET /alert/{id}", "POST /progress"],
                }))
            }
            ("POST", "/alert") => Some(self.alert(&request.body)),
            ("POST", "/progress") => Some(self.progress(&request.body)),
            ("GET", p) if p.starts_with("/alert/") => Some(self.alert_state(&p["/alert/".len()..])),
            _ => None,
        }
    }

    // ───────────────────────────── alert results (buttons) ─────────────────────────────

    fn alert_state(&self, id: &str) -> Response {
        #[derive(Serialize)]
        struct State<'a> {
            id: &'a str,
            state: String,
            button: Option<String>,
        }
        match self.results.borrow().get(id) {
            Some(r) => Response::json(200, &State { id, state: r.state.borrow().clone(), button: r.button.borrow().clone() }),
            None => Response::error(404, "No alert with that id (alerts are kept for 15 minutes)."),
        }
    }

    fn new_result(&self) -> (String, Rc<AlertResult>) {
        self.results.borrow_mut().retain(|_, r| r.created.elapsed() <= PROGRESS_EXPIRY);
        let id = random_id();
        let result = Rc::new(AlertResult { state: RefCell::new("open".into()), button: RefCell::new(None), created: Instant::now() });
        self.results.borrow_mut().insert(id.clone(), result.clone());
        (id, result)
    }

    // ───────────────────────────── alerts ─────────────────────────────

    fn alert(&self, body: &Value) -> Response {
        let Some(title) = text(body, "title", 120).filter(|t| !t.is_empty()) else { return Response::error(400, "\"title\" is required.") };
        if !self.allow_alert() {
            return Response::error(429, &format!("More than {MAX_ALERTS_PER_MINUTE} alerts a minute; slow down."));
        }
        let url = match link(body) {
            Ok(u) => u,
            Err(e) => return e,
        };
        let buttons = match buttons(body) {
            Ok(b) => b,
            Err(e) => return e,
        };

        let color = color(body).unwrap_or_else(colors::blue);
        // With buttons, give people time to choose.
        let seconds = number(body, "seconds").map(|s| s.clamp(1.0, 60.0)).unwrap_or(if buttons.is_empty() { 5.0 } else { 12.0 });
        let sticky = body.get("sticky").and_then(Value::as_bool) == Some(true);
        let (id, result) = self.new_result();
        let glyph = glyph(body);
        let message = text(body, "message", 300).unwrap_or_default();
        let has_buttons = !buttons.is_empty();
        let manager = self.manager.clone();
        let (r1, r2, r3) = (result.clone(), result.clone(), result.clone());
        let id_key = id.clone();

        let alert = Alert::cyclic(move |weak| {
            let content = move || {
                let icon = ui::glyph_circle(glyph, color, ICON_SIZE, 22.0);
                if buttons.is_empty() {
                    return ui::alert_view(icon, &title, &message, None, None);
                }
                // Buttons go in a row of wide capsules under the alert, as in the iPhone's expanded prompts.
                let actions = buttons
                    .iter()
                    .enumerate()
                    .map(|(i, (button_id, label, button_url))| {
                        let primary = i == buttons.len() - 1; // the last one is the main action
                        let (r, w, m, button_id, button_url) = (r1.clone(), weak.clone(), manager.clone(), button_id.clone(), button_url.clone());
                        ui::action_button(key(&format!("api.{id_key}.{i}")), label, move || {
                            *r.state.borrow_mut() = "clicked".into();
                            *r.button.borrow_mut() = Some(button_id.clone());
                            if let Some(u) = &button_url {
                                crate::interop::shell_open(u);
                            }
                            if let Some(a) = w.upgrade() {
                                m.dismiss_alert(Some(&a));
                            }
                        }, primary.then_some(color))
                    })
                    .collect();
                ui::action_alert_view(icon, &title, &message, actions)
            };
            Alert::new(content)
                .size(ALERT_WIDTH, if has_buttons { ACTION_ALERT_HEIGHT } else { ALERT_HEIGHT })
                .keyline(Some(color))
                .seconds(seconds)
                .sticky_until_hover(sticky)
                .on_click(move || {
                    if *r2.state.borrow() == "open" {
                        *r2.state.borrow_mut() = "clicked".into();
                    }
                    if let Some(u) = &url {
                        crate::interop::shell_open(u);
                    }
                })
                .on_dismissed(move || {
                    if *r3.state.borrow() == "open" {
                        *r3.state.borrow_mut() = "dismissed".into();
                    }
                })
        });
        self.manager.show_alert(alert);

        #[derive(Serialize)]
        struct Ok<'a> {
            ok: bool,
            id: &'a str,
        }
        Response::json(200, &Ok { ok: true, id: &id })
    }

    fn allow_alert(&self) -> bool {
        let mut recent = self.recent_alerts.borrow_mut();
        while recent.front().is_some_and(|t| t.elapsed() > Duration::from_secs(60)) {
            recent.pop_front();
        }
        if recent.len() >= MAX_ALERTS_PER_MINUTE {
            return false;
        }
        recent.push_back(Instant::now());
        true
    }

    // ───────────────────────────── progress ─────────────────────────────

    fn progress(&self, body: &Value) -> Response {
        let Some(id) = text(body, "id", 64).filter(|t| !t.is_empty()) else { return Response::error(400, "\"id\" is required.") };

        if body.get("done").and_then(Value::as_bool) == Some(true) {
            let finished = self.progress.borrow_mut().remove(&id);
            if let Some(finished) = finished {
                finished.activity.hide();
                if let Some(message) = text(body, "message", 120).filter(|m| !m.is_empty()) {
                    let detail = text(body, "detail", 300).unwrap_or_default();
                    self.manager.show_alert(Alert::new(move || ui::alert_view(ui::glyph_circle(icons::CHECK, colors::green(), ICON_SIZE, 22.0), &message, &detail, None, None)).seconds(4.0).rc());
                }
            }
            return Response::ok();
        }

        if !self.progress.borrow().contains_key(&id) {
            if self.progress.borrow().len() >= MAX_PROGRESS {
                return Response::error(429, &format!("At most {MAX_PROGRESS} progress items at once."));
            }
            let item = self.create(&id);
            self.progress.borrow_mut().insert(id.clone(), item);
        }

        let value = number(body, "value").map(|v| v.clamp(0.0, 1.0));
        let mut items = self.progress.borrow_mut();
        let item = items.get_mut(&id).unwrap();
        {
            let mut v = item.view.borrow_mut();
            if let Some(c) = color(body) {
                v.color = c;
            }
            if let Some(t) = text(body, "title", 120) {
                v.title = t;
            }
            if let Some(d) = text(body, "detail", 200) {
                v.detail = d;
            }
            v.value = value;
        }
        item.updated = Instant::now();
        let activity = item.activity.clone();
        drop(items);
        activity.show();
        Response::ok()
    }

    fn create(&self, id: &str) -> ProgressItem {
        let view = Rc::new(RefCell::new(ProgressView { title: id.to_string(), detail: String::new(), value: None, color: colors::blue() }));
        let (a, b, c, d) = (view.clone(), view.clone(), view.clone(), view.clone());
        let activity = AddonActivity::new(
            self.manager.clone(),
            &format!("API: {id}"),
            "Api",
            14,
            Box::new(move || {
                let v = a.borrow();
                icons::ring_on_track(v.value.unwrap_or(0.25), 18.0, 2.6, v.color)
            }),
            Box::new(move || {
                let v = b.borrow();
                let percent = v.value.map_or("…".to_string(), |p| format!("{:.0}%", p * 100.0));
                ui::digits(percent, 14.0, SEMIBOLD, v.color)
            }),
            Box::new(move || {
                let v = c.borrow();
                icons::ring_on_track(v.value.unwrap_or(0.25), 22.0, 2.4, v.color)
            }),
            Box::new(move || {
                let v = d.borrow();
                let fill = El::empty().w(BAR_WIDTH * v.value.unwrap_or(0.0)).h(6.0).radius(3.0).bg(v.color).halign(Align::Start);
                let track = El::boxed(vec![fill]).size(BAR_WIDTH, 6.0).radius(3.0).bg(colors::track()).margin(0.0, 10.0, 0.0, 0.0).halign(Align::Start);
                El::vstack(vec![ui::text(v.title.as_str(), 15.0, SEMIBOLD, v.color), ui::text(v.detail.as_str(), 13.0, NORMAL, colors::grey()), track]).margin(18.0, 16.0, 18.0, 14.0)
            }),
            230.0,
            100.0,
            None,
        );
        ProgressItem { activity, view, updated: Instant::now() }
    }

    /// A script that crashed never sends "done"; don't leave its progress in the island forever.
    fn expire_stale(&self) {
        let stale: Vec<String> = self.progress.borrow().iter().filter(|(_, p)| p.updated.elapsed() > PROGRESS_EXPIRY).map(|(k, _)| k.clone()).collect();
        for id in stale {
            if let Some(item) = self.progress.borrow_mut().remove(&id) {
                item.activity.hide();
            }
        }
    }
}

/// Runs the request on the UI thread and waits for the answer (Dispatcher.Invoke).
fn handle_on_ui(request: Request) -> Option<Response> {
    let (tx, rx) = std::sync::mpsc::channel();
    let request = SendRequest(request);
    crate::rt::post(move || {
        let request = request;
        let answer = API.with(|a| a.borrow().clone()).and_then(|api| api.handle(&request.0));
        let _ = tx.send(answer);
    });
    rx.recv_timeout(Duration::from_secs(10)).unwrap_or_else(|_| Some(Response::error(500, "The island didn't answer in time.")))
}

struct SendRequest(Request);

// ───────────────────────────── input ─────────────────────────────

/// A string field, trimmed and cut at `max` characters (UTF-16 units, as .NET counts) with an ellipsis.
fn text(body: &Value, name: &str, max: usize) -> Option<String> {
    let value = body.get(name)?.as_str()?.trim();
    let units: Vec<u16> = value.encode_utf16().collect();
    if units.len() > max {
        Some(format!("{}…", String::from_utf16_lossy(&units[..max])))
    } else {
        Some(value.to_string())
    }
}

fn number(body: &Value, name: &str) -> Option<f64> {
    body.get(name)?.as_f64()
}

fn glyph(body: &Value) -> &'static str {
    let name = text(body, "icon", 20).unwrap_or_default().to_lowercase();
    ICON_NAMES.iter().find(|(n, _)| *n == name).map_or(ICON_NAMES[0].1, |(_, g)| g)
}

fn color(body: &Value) -> Option<Color> {
    let value = text(body, "color", 20)?;
    named_color(&value).or_else(|| if value.starts_with('#') && value.chars().count() == 7 { Color::hex(&value) } else { None })
}

/// Only web links: a click on an alert should never be able to start a program.
fn link(body: &Value) -> Result<Option<String>, Response> {
    let Some(url) = text(body, "url", 2000) else { return Ok(None) };
    let lower = url.to_lowercase();
    let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"));
    match rest {
        Some(r) if !r.is_empty() && !r.starts_with('/') && !url.contains(char::is_whitespace) => Ok(Some(url)),
        _ => Err(Response::error(400, "\"url\" must be an http or https link.")),
    }
}

type Button = (String, String, Option<String>);

/// Up to two buttons: {"id":"open","label":"Open log","url":"https://…"} (url optional).
fn buttons(body: &Value) -> Result<Vec<Button>, Response> {
    let Some(list) = body.get("buttons") else { return Ok(Vec::new()) };
    let Some(list) = list.as_array().filter(|l| l.len() <= 2) else { return Err(Response::error(400, "\"buttons\" must be a list of at most 2 buttons.")) };
    let mut out = Vec::new();
    for b in list {
        let Some(label) = b.is_object().then(|| text(b, "label", 20)).flatten().filter(|l| !l.is_empty()) else {
            return Err(Response::error(400, "Every button needs a \"label\" (up to 20 characters)."));
        };
        let url = link(b)?;
        out.push((text(b, "id", 40).unwrap_or_else(|| label.clone()), label, url));
    }
    Ok(out)
}

/// Ten random hex digits (the start of a GUID in the C# app).
fn random_id() -> String {
    let mut bytes = [0u8; 5];
    unsafe {
        let _ = windows::Win32::Security::Cryptography::BCryptGenRandom(None, &mut bytes, windows::Win32::Security::Cryptography::BCRYPT_USE_SYSTEM_PREFERRED_RNG);
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn texts_are_trimmed_and_cut() {
        let body = json!({ "title": "  Backup done  ", "long": "x".repeat(130) });
        assert_eq!(text(&body, "title", 120).as_deref(), Some("Backup done"));
        assert_eq!(text(&body, "long", 120).unwrap().chars().count(), 121);
        assert_eq!(text(&body, "missing", 10), None);
        assert_eq!(text(&json!({ "title": 5 }), "title", 10), None);
    }

    #[test]
    fn only_web_links() {
        assert!(link(&json!({ "url": "https://example.com/x" })).unwrap().is_some());
        assert!(link(&json!({ "url": "HTTP://example.com" })).unwrap().is_some());
        assert!(link(&json!({ "url": "file:///C:/Windows/notepad.exe" })).is_err());
        assert!(link(&json!({ "url": "calc.exe" })).is_err());
        assert!(link(&json!({})).unwrap().is_none());
    }

    #[test]
    fn buttons_are_checked() {
        assert_eq!(buttons(&json!({ "buttons": [{ "label": "Retry" }] })).unwrap()[0].0, "Retry");
        assert!(buttons(&json!({ "buttons": [{ "id": "a" }] })).is_err());
        assert!(buttons(&json!({ "buttons": [{ "label": "1" }, { "label": "2" }, { "label": "3" }] })).is_err());
    }

    #[test]
    fn colors_and_icons_by_name() {
        assert_eq!(color(&json!({ "color": "Green" })), Some(colors::green()));
        assert_eq!(color(&json!({ "color": "#112233" })), Some(Color::rgb(0x11, 0x22, 0x33)));
        assert_eq!(color(&json!({ "color": "#11223" })), None);
        assert_eq!(glyph(&json!({ "icon": "SUCCESS" })), "\u{E73E}");
        assert_eq!(glyph(&json!({ "icon": "nope" })), "\u{E946}");
        assert_eq!(random_id().len(), 10);
    }
}
