//! Watches Windows toast notifications (Action Center) and raises the ones from allow-listed apps such as
//! Discord. Polls a little more than once a second, because the NotificationChanged event is only delivered to
//! packaged apps with a background task.

use crate::ui::image::{self, Bitmap, Decode};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows::Foundation::Size;
use windows::UI::Notifications::Management::{UserNotificationListener, UserNotificationListenerAccessStatus as Access};
use windows::UI::Notifications::{KnownNotificationBindings, NotificationKinds, UserNotification};

/// A notification to show. `app`: the app it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct Incoming {
    pub app: String,
    pub title: String,
    pub body: String,
    pub logo: Option<Bitmap>,
}

/// What the poller needs from the settings (updated when they change).
#[derive(Clone, Default)]
pub struct Filter {
    pub enabled: bool,
    pub apps: Vec<String>,
}

/// Notifications older than this never pop out of the island (seconds).
const MAX_AGE: i64 = 120;

/// Asks Windows for access, then polls; `received` runs on the UI thread, `status` once with the access result
/// ("Allowed", "Denied", "Unspecified", or "Unavailable (…)").
pub fn start(filter: Arc<Mutex<Filter>>, received: impl Fn(Incoming) + 'static, status: impl Fn(String) + 'static) {
    let received = crate::rt::Handle::new(received);
    let status = crate::rt::Handle::new(status);
    crate::rt::background(move || {
        let listener = match UserNotificationListener::Current() {
            Ok(l) => l,
            Err(e) => {
                di4w_core::log::error_with("Notification listener unavailable", &e);
                status.send(format!("Unavailable ({})", e.code()));
                return;
            }
        };
        let access = listener.RequestAccessAsync().and_then(|op| op.join()).or_else(|_| listener.GetAccessStatus());
        let text = match access {
            Ok(Access::Allowed) => "Allowed".to_string(),
            Ok(Access::Denied) => "Denied".to_string(),
            Ok(_) => "Unspecified".to_string(),
            Err(e) => format!("Unavailable ({})", e.code()),
        };
        di4w_core::log::info(format!("Notification access: {text}"));
        status.send(text.clone());
        if text != "Allowed" {
            return;
        }
        let mut seen: HashSet<u32> = HashSet::new();
        // Don't replay notifications that were already sitting in Action Center.
        if let Ok(list) = listener.GetNotificationsAsync(NotificationKinds::Toast).and_then(|op| op.join()) {
            for n in &list {
                if let Ok(id) = n.Id() {
                    seen.insert(id);
                }
            }
        }
        let mut logos: HashMap<String, Option<Bitmap>> = HashMap::new();
        loop {
            std::thread::sleep(Duration::from_millis(800));
            let list = match listener.GetNotificationsAsync(NotificationKinds::Toast).and_then(|op| op.join()) {
                Ok(l) => l,
                Err(e) => {
                    di4w_core::log::error_with("Notification poll failed", e);
                    continue;
                }
            };
            let mut all: Vec<(i64, UserNotification)> = (&list).into_iter().map(|n| (n.CreationTime().map(|t| t.UniversalTime).unwrap_or(0), n)).collect();
            all.sort_by_key(|(t, _)| *t);
            let now = now_ticks();
            let mut ids = HashSet::new();
            for (created, n) in all {
                let Ok(id) = n.Id() else { continue };
                ids.insert(id);
                if !seen.insert(id) {
                    continue;
                }
                // Old news, whatever the list says: Windows sometimes returns an empty list for a moment (after sleep,
                // locking, or Explorer restarting), after which everything in Action Center would look new.
                if (now - created) / 10_000_000 > MAX_AGE {
                    continue;
                }
                let f = filter.lock().unwrap_or_else(|e| e.into_inner()).clone();
                if !f.enabled {
                    continue;
                }
                if let Some(parsed) = parse(&n, &f, &mut logos) {
                    received.send(parsed);
                }
            }
            // Forget dismissed notifications so the set doesn't grow forever.
            seen.retain(|id| ids.contains(id));
        }
    });
}

fn now_ticks() -> i64 {
    let ft = unsafe { windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime() };
    ((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64
}

fn parse(n: &UserNotification, filter: &Filter, logos: &mut HashMap<String, Option<Bitmap>>) -> Option<Incoming> {
    let info = n.AppInfo().ok()?.DisplayInfo().ok()?;
    let app = info.DisplayName().ok()?.to_string();
    let lower = app.to_lowercase();
    if !filter.apps.iter().any(|a| lower.contains(&a.to_lowercase())) {
        return None;
    }
    let texts: Vec<String> = n
        .Notification()
        .and_then(|x| x.Visual())
        .and_then(|v| v.GetBinding(&KnownNotificationBindings::ToastGeneric()?))
        .and_then(|b| b.GetTextElements())
        .map(|list| (&list).into_iter().filter_map(|t| t.Text().ok().map(|s| s.to_string())).filter(|t| !t.trim().is_empty()).collect())
        .unwrap_or_default();
    let title = texts.first().cloned().unwrap_or_else(|| app.clone());
    let body = texts.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
    let logo = logos
        .entry(app.clone())
        .or_insert_with(|| {
            let reference = info.GetLogo(Size { Width: 96.0, Height: 96.0 }).ok()?;
            let bytes = crate::activities::media::service::read_stream(&reference.into())?;
            match image::decode(&bytes, Decode::default()) {
                Ok(b) => Some(b),
                Err(e) => {
                    di4w_core::log::error_with(format!("No logo for {app}"), e);
                    None
                }
            }
        })
        .clone();
    Some(Incoming { app, title, body, logo })
}
