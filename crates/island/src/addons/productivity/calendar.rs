//! "Meeting in 5 min" with a Join button for Teams/Zoom/Meet/Webex links. Reads calendar links (.ics) that
//! Outlook, Google Calendar and iCloud can publish; recurring meetings and time zones are handled.

use super::ics::{self, Meeting};
use crate::addons::{Addon, Base, Context};
use crate::model::Alert;
use crate::rt::Timer;
use crate::ui::colors;
use crate::ui::el::key;
use crate::ui::{self, ACTION_ALERT_HEIGHT, ALERT_HEIGHT, ALERT_WIDTH, ICON_SIZE};
use chrono::{DateTime, Utc};
use di4w_core::{AddonSettings, l};
use regex::Regex;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::Duration;

/// A meeting ready to announce: its title (or "Meeting") and Join link.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub key: String,
    pub title: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub location: Option<String>,
    pub join_url: Option<String>,
}

/// Occurrences between `from` and `to`, with their Join links.
pub fn parse(text: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<Entry> {
    ics::parse(text, from, to)
        .into_iter()
        .map(|m: Meeting| Entry {
            join_url: join_link(&format!("{} {} {}", m.location.clone().unwrap_or_default(), m.url, m.description)),
            key: m.key,
            title: m.title.unwrap_or_else(|| l::t("Meeting").to_string()),
            start: m.start,
            end: m.end,
            location: m.location,
        })
        .collect()
}

/// The first online-meeting link in the text.
pub fn join_link(text: &str) -> Option<String> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(r#"(?i)https://[^\s"<>]*(teams\.microsoft\.com/l/meetup-join|teams\.live\.com/meet|zoom\.us/(j|my|w)/|meet\.google\.com/|webex\.com/(meet|join)|whereby\.com/)[^\s"<>]*"#).unwrap()
    });
    pattern.find(text).map(|m| m.as_str().trim_end_matches(['.', ',', ')', '>']).to_string())
}

pub struct Calendar {
    base: Base,
    announced: RefCell<HashSet<String>>,
    meetings: RefCell<Vec<Entry>>,
    loaded_feeds: RefCell<String>,
    timers: RefCell<Vec<Timer>>,
}

impl Calendar {
    pub fn new(ctx: &Rc<Context>) -> Rc<Calendar> {
        Rc::new(Calendar { base: Base::new(ctx, "Calendar", "Calendar"), announced: Default::default(), meetings: Default::default(), loaded_feeds: Default::default(), timers: Default::default() })
    }

    fn refresh(self: &Rc<Self>) {
        let feeds = self.base.options().calendar_feeds;
        *self.loaded_feeds.borrow_mut() = feeds.join("\n");
        let me = Rc::downgrade(self);
        let count = feeds.len();
        crate::rt::spawn(
            move || {
                let now = Utc::now();
                let (from, to) = (now - chrono::Duration::minutes(5), now + chrono::Duration::days(1));
                let mut meetings = Vec::new();
                let (mut failed, mut failure) = (0, None);
                for feed in feeds {
                    let trimmed = feed.trim();
                    let url = match trimmed.split_once("://") {
                        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("webcal") || scheme.eq_ignore_ascii_case("webcals") => format!("https://{rest}"),
                        _ => trimmed.to_string(),
                    };
                    match fetch(&url) {
                        // Parsing and expanding recurrences can take a moment for big calendars; off the UI thread.
                        Ok(text) => meetings.extend(parse(&text, from, to)),
                        Err(e) => {
                            di4w_core::log::info(format!("Calendar feed failed: {e}"));
                            failed += 1;
                            failure = Some(e);
                        }
                    }
                }
                (meetings, failed, failure)
            },
            move |(meetings, failed, failure)| {
                if let Some(c) = me.upgrade() {
                    if !c.base.running() {
                        return;
                    }
                    *c.meetings.borrow_mut() = meetings;
                    c.report(count, failed, failure);
                    c.check();
                }
            },
        );
    }

    fn report(&self, feeds: usize, failed: usize, failure: Option<String>) {
        if feeds == 0 {
            self.base.set_status(Some(l::t("Add a calendar link below to get meeting reminders.").into()), true);
        } else if failed > 0 {
            let failure = failure.unwrap_or_default();
            self.base.set_status(
                Some(if feeds == 1 {
                    l::f("Couldn't load your calendar: {0}", &[&failure])
                } else {
                    l::f("{0} of {1} calendar links couldn't be loaded: {2}", &[&failed, &feeds, &failure])
                }),
                true,
            );
        } else {
            let now = Utc::now();
            let next = self.meetings.borrow().iter().filter(|m| m.start > now).min_by_key(|m| m.start).cloned();
            match next {
                None => self.base.set_status(Some(l::t("No meetings in the next 24 hours.").into()), false),
                Some(m) => self.base.set_status(Some(l::f("Next: {0} at {1:t}", &[&m.title, &crate::activities::short_time(m.start.with_timezone(&chrono::Local))])), false),
            }
        }
    }

    fn check(self: &Rc<Self>) {
        // Calendar links were added or removed in the settings: load them now, not at the next refresh.
        let feeds = self.base.options().calendar_feeds.join("\n");
        if feeds != *self.loaded_feeds.borrow() {
            self.refresh();
            return;
        }
        let now = Utc::now();
        let lead = chrono::Duration::minutes(self.base.options().calendar_reminder_minutes.max(1) as i64);
        let mut due: Vec<Entry> = self.meetings.borrow().clone();
        due.sort_by_key(|m| m.start);
        for m in due {
            if m.start - now > lead || now > m.start + chrono::Duration::minutes(2) || !self.announced.borrow_mut().insert(m.key.clone()) {
                continue;
            }
            self.announce(&m, now);
        }
    }

    fn announce(&self, m: &Entry, now: DateTime<Utc>) {
        let minutes = ((m.start - now).num_seconds() as f64 / 60.0).ceil() as i64;
        let when = if minutes <= 0 { l::t("Starting now").to_string() } else { l::f("In {0} min", &[&minutes]) };
        let local = |t: DateTime<Utc>| crate::activities::short_time(t.with_timezone(&chrono::Local));
        let times = format!("{}–{}", local(m.start), local(m.end));
        let place = match &m.location {
            Some(loc) if !loc.trim().is_empty() && m.join_url.is_none() => format!(" · {loc}"),
            _ => String::new(),
        };
        let detail = format!("{when} · {times}{place}");
        let title = m.title.clone();
        let join = m.join_url.clone();
        let manager = self.base.ctx.manager.clone();
        let has_join = join.is_some();
        // A meeting with an online link gets a wide Join button under it, like the iPhone's expanded prompts.
        let alert = Alert::cyclic(move |weak| {
            Alert::new(move || {
                let icon = ui::glyph_circle("\u{E787}", colors::red(), ICON_SIZE, 22.0);
                match &join {
                    None => ui::alert_view(icon, &title, &detail, None, None),
                    Some(url) => {
                        let (url, w, m) = (url.clone(), weak.clone(), manager.clone());
                        let button = ui::action_button(key("calendar.join"), l::t("Join"), move || {
                            crate::interop::shell_open(&url);
                            if let Some(a) = w.upgrade() {
                                m.dismiss_alert(Some(&a));
                            }
                        }, Some(colors::green()));
                        ui::action_alert_view(icon, &title, &detail, vec![button])
                    }
                }
            })
            .size(ALERT_WIDTH, if has_join { ACTION_ALERT_HEIGHT } else { ALERT_HEIGHT })
            .keyline(Some(colors::red()))
            .seconds(10.0)
            .sticky_until_hover(true)
        });
        self.base.show(alert);
    }
}

fn fetch(url: &str) -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(60))).build().into();
    let mut response = agent.get(url).header("User-Agent", di4w_core::updates::USER_AGENT).call().map_err(|e| e.to_string())?;
    response.body_mut().with_config().limit(50 * 1024 * 1024).read_to_string().map_err(|e| e.to_string())
}

impl Addon for Calendar {
    fn base(&self) -> &Base {
        &self.base
    }

    fn enabled(&self, o: &AddonSettings) -> bool {
        o.calendar
    }

    fn start(self: Rc<Self>) -> Result<(), String> {
        let (a, b) = (Rc::downgrade(&self), Rc::downgrade(&self));
        *self.timers.borrow_mut() = vec![
            Timer::every(Duration::from_secs(15 * 60), move || {
                if let Some(c) = a.upgrade() {
                    c.refresh();
                }
            }),
            Timer::every(Duration::from_secs(20), move || {
                if let Some(c) = b.upgrade() {
                    c.check();
                }
            }),
        ];
        self.refresh();
        Ok(())
    }

    fn stop(&self) {
        self.timers.borrow_mut().clear();
        self.meetings.borrow_mut().clear();
    }

    fn preview(self: Rc<Self>) {
        let now = Utc::now();
        let sample = Entry {
            key: "preview".into(),
            title: l::t("Team meeting").into(),
            start: now + chrono::Duration::minutes(5),
            end: now + chrono::Duration::minutes(35),
            location: None,
            join_url: None,
        };
        self.announce(&sample, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    // A weekly stand-up with a Teams link, a cancelled meeting, and an all-day event.
    const ICS: &str = "BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//Test//EN\nBEGIN:VEVENT\nUID:standup\nDTSTART:20261005T080000Z\nDTEND:20261005T081500Z\nRRULE:FREQ=WEEKLY;BYDAY=MO,WE\nSUMMARY:Stand-up\nDESCRIPTION:Join: https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc%40thread.v2/0?context=x\nEND:VEVENT\nBEGIN:VEVENT\nUID:cancelled\nDTSTART:20261012T100000Z\nDTEND:20261012T110000Z\nSUMMARY:Old planning\nSTATUS:CANCELLED\nEND:VEVENT\nBEGIN:VEVENT\nUID:holiday\nDTSTART;VALUE=DATE:20261012\nDTEND;VALUE=DATE:20261013\nSUMMARY:Day off\nEND:VEVENT\nEND:VCALENDAR\n";

    fn monday() -> Vec<Entry> {
        parse(ICS, Utc.with_ymd_and_hms(2026, 10, 12, 0, 0, 0).unwrap(), Utc.with_ymd_and_hms(2026, 10, 13, 0, 0, 0).unwrap())
    }

    #[test]
    fn expands_recurring_meetings_in_the_window() {
        // Monday 12 October 2026, whole day: one stand-up occurrence.
        let meetings = monday();
        assert_eq!(meetings.len(), 1);
        let standup = &meetings[0];
        assert_eq!(standup.title, "Stand-up");
        assert_eq!(standup.start, Utc.with_ymd_and_hms(2026, 10, 12, 8, 0, 0).unwrap());
        assert!(standup.join_url.as_deref().unwrap().starts_with("https://teams.microsoft.com/l/meetup-join/"));
    }

    #[test]
    fn skips_cancelled_and_all_day_events() {
        assert!(!monday().iter().any(|m| m.title == "Old planning" || m.title == "Day off"));
    }

    #[test]
    fn finds_online_meeting_links() {
        for (text, expected) in [
            ("Location: https://zoom.us/j/123456789?pwd=abc.", Some("https://zoom.us/j/123456789?pwd=abc")),
            ("<https://meet.google.com/abc-defg-hij>", Some("https://meet.google.com/abc-defg-hij")),
            ("Room 4.12", None),
            ("https://example.com/agenda", None),
        ] {
            assert_eq!(join_link(text).as_deref(), expected, "{text}");
        }
    }
}
