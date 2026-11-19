//! Turns incoming notifications into island alerts: chat apps get their brand color and clicking switches to the
//! right app; incoming calls become a large call card with decline / answer, like the iPhone's.

pub mod service;

use crate::model::{Alert, Manager};
use crate::ui::colors::{self, Color};
use crate::ui::el::{Align, El, key};
use crate::ui::image::Image;
use crate::ui::{self, ALERT_HEIGHT, ALERT_WIDTH, ICON_SIZE, SEMIBOLD, icons};
use di4w_core::{AppSettings, ChatApps, l};
use regex::Regex;
use service::Incoming;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;

pub struct Presenter {
    manager: Rc<Manager>,
    settings: Rc<RefCell<AppSettings>>,
}

impl Presenter {
    pub fn new(manager: Rc<Manager>, settings: Rc<RefCell<AppSettings>>) -> Presenter {
        Presenter { manager, settings }
    }

    pub fn show(&self, n: Incoming) {
        let preset = ChatApps::find(&n.app);
        let color = preset.and_then(|p| Color::hex(p.color)).unwrap_or_else(colors::discord_blurple);
        let logo = n.logo.clone().map(Image::new);
        let processes: Vec<String> = match preset {
            Some(p) => p.processes.iter().map(|s| s.to_string()).collect(),
            None => vec![n.app.replace(' ', "")],
        };
        let open = Rc::new(move || {
            for p in &processes {
                if crate::interop::is_running(p) {
                    crate::interop::focus_process(p);
                    return;
                }
            }
        });

        if self.settings.borrow().addons.calls && preset.is_some_and(|p| p.has_calls) && is_call(&n.title, &n.body) {
            self.show_call(&n, preset.unwrap().name, open);
            return;
        }

        let short = n.body.is_empty() || n.body.chars().count() < 40;
        let (title, body) = (n.title.clone(), n.body.clone());
        let o = open.clone();
        let alert = Alert::new(move || {
            let icon = match &logo {
                Some(img) => ui::image(Some(img.id), ICON_SIZE, 12.0),
                None => ui::glyph_circle(icons::CHAT, color, ICON_SIZE, 22.0),
            };
            ui::alert_view(icon, &title, &body, None, None)
        })
        .size(ALERT_WIDTH, if short { ALERT_HEIGHT } else { 100.0 })
        .seconds(5.0)
        .on_click(move || o())
        .keyline(Some(color));
        self.manager.show_alert(alert.rc());
    }

    fn show_call(&self, n: &Incoming, app: &'static str, open: Rc<dyn Fn()>) {
        // The caller is usually the title; some apps put "Incoming call" there and the name in the body.
        let caller = if call_pattern().is_match(&n.title) && !n.body.trim().is_empty() { n.body.clone() } else { n.title.clone() };
        let manager = self.manager.clone();
        let alert = Alert::cyclic(move |weak| {
            let (w1, w2, m1, m2, o) = (weak.clone(), weak.clone(), manager.clone(), manager.clone(), open.clone());
            let caller2 = caller.clone();
            // Laid out like the iPhone's incoming call: the caller's initials, the name, and the red and green buttons.
            Alert::new(move || {
                let (w1, w2, m1, m2, o) = (w1.clone(), w2.clone(), m1.clone(), m2.clone(), o.clone());
                let decline = ui::circle_button(key("call.decline"), ui::glyph("\u{E778}", 20.0, colors::on_color()), colors::red(), 48.0, move || {
                    if let Some(a) = w1.upgrade() {
                        m1.dismiss_alert(Some(&a));
                    }
                }, Some(l::t("Decline").into()));
                let answer = ui::circle_button(key("call.answer"), ui::glyph("\u{E717}", 20.0, colors::on_color()), colors::green(), 48.0, move || {
                    o();
                    if let Some(a) = w2.upgrade() {
                        m2.dismiss_alert(Some(&a));
                    }
                }, Some(l::f("Answer in {0}", &[&app])))
                .margin(10.0, 0.0, 0.0, 0.0);
                let buttons = El::hstack(vec![decline, answer]);
                ui::alert_view(avatar(&caller2), &caller2, &l::f("Incoming call · {0}", &[&app]), None, Some(buttons))
            })
            .seconds(30.0)
            .keyline(Some(colors::green()))
            .on_click_opt(Some(open.clone()))
        });
        self.manager.show_alert(alert);
    }
}

/// A contact picture the way iOS draws one without a photo: initials on a grey gradient circle.
pub fn avatar(name: &str) -> El {
    let initials = initials(name);
    let child = if initials.is_empty() {
        ui::glyph("\u{E77B}", 24.0, colors::on_color()) // Contact
    } else {
        ui::text(initials, 21.0, SEMIBOLD, colors::on_color()).align(Align::Center, Align::Center)
    };
    El::boxed(vec![child])
        .size(ICON_SIZE, ICON_SIZE)
        .radius(ICON_SIZE / 2.0)
        .bg(crate::ui::el::Brush::Vertical(Color::rgb(0xA5, 0xA9, 0xB4), Color::rgb(0x84, 0x88, 0x93)))
}

/// "Anna de Vries" -> "AV", "sam" -> "S"; nothing for names without letters (a phone number).
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split(' ').filter(|w| w.chars().next().is_some_and(char::is_alphabetic)).collect();
    let first = |w: &str| w.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    match words.as_slice() {
        [] => String::new(),
        [one] => first(one),
        [a, .., b] => format!("{}{}", first(a), first(b)),
    }
}

fn call_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(concat!(
            r"(?i)incoming (voice |video |audio )?call|is calling|calling you|wants to (video )?call|ringing|",
            r"inkomende? (spraak|video)?\s?(oproep|gesprek)|belt (je|u)|wil je bellen|video-?oproep|spraakoproep|",
            r"eingehender (sprach|video)?\s?anruf|ruft (dich|sie) an|videoanruf|sprachanruf|",
            r"appel (vocal |vidéo |video )?entrant|vous appelle|t'appelle|appel vidéo",
        ))
        .unwrap()
    })
}

/// Call notifications from the supported apps, in English, Dutch, German and French (apps write them in the
/// Windows language).
pub fn is_call(title: &str, body: &str) -> bool {
    call_pattern().is_match(&format!("{title} {body}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_calls() {
        for (title, body) in [
            ("Anna", "Incoming call"),
            ("Incoming video call", "Sam"),
            ("Jan", "is calling you"),
            ("Inkomende video-oproep", "Jan"),
            ("Mark", "belt je"),
            ("Lisa", "Inkomend spraakgesprek"),
        ] {
            assert!(is_call(title, body), "{title} / {body}");
        }
    }

    #[test]
    fn ignores_ordinary_messages() {
        for (title, body) in [("#general", "Anna: are we still calling it a day?"), ("Jan", "I called you earlier"), ("Mark", "Missed call")] {
            assert!(!is_call(title, body), "{title} / {body}");
        }
    }

    #[test]
    fn callers_get_initials_like_ios_contacts() {
        assert_eq!(initials("Anna de Vries"), "AV");
        assert_eq!(initials("sam"), "S");
        assert_eq!(initials("+31 6 12345678"), "");
    }

    #[test]
    fn finds_chat_app_presets() {
        for (display, preset) in [("Discord", "Discord"), ("Telegram Desktop", "Telegram"), ("Microsoft Teams (work or school)", "Microsoft Teams"), ("whatsapp", "WhatsApp")] {
            assert_eq!(ChatApps::find(display).map(|a| a.name), Some(preset));
        }
        assert!(ChatApps::find("Spotify").is_none());
        assert!(ChatApps::find("Steam").is_none());
    }
}
