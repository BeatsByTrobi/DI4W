//! Translations for both apps. Text is written in English in the code and looked up here:
//!
//! ```ignore
//! l::t("Low battery")              // "Batterij raakt leeg" in Dutch
//! l::f("In {0} min", &[&minutes])  // "Over 5 min"
//! ```
//!
//! The tables live in `strings/<lang>.json`, keyed by the English text. Always call these as `l::t("…")` and
//! `l::f("…", …)` with the English text as a literal (in the settings app's `.slint` files: `L.t("…")`), so the
//! translation tests can find every text: each needs an entry in every language, and an entry whose English text
//! is gone from the code fails the tests too.
//!
//! Placeholders are `{0}`, `{1}`… A format after a colon (`{1:t}` time, `{0:0}` whole number, `{1:0.#}` one
//! decimal) documents how the caller formats the value; the value itself is passed already formatted.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The languages besides English, with their name in that language (for the settings).
pub const LANGUAGES: [(&str, &str); 3] = [("nl", "Nederlands"), ("de", "Deutsch"), ("fr", "Français")];

const CODES: [&str; 4] = ["en", "nl", "de", "fr"];
const SOURCES: [&str; 3] = [include_str!("../strings/nl.json"), include_str!("../strings/de.json"), include_str!("../strings/fr.json")];

/// Index into [`CODES`]; 0 is English (no table).
static CURRENT: AtomicUsize = AtomicUsize::new(0);

fn tables() -> &'static [HashMap<String, String>; 3] {
    static TABLES: OnceLock<[HashMap<String, String>; 3]> = OnceLock::new();
    TABLES.get_or_init(|| SOURCES.map(|json| serde_json::from_str(json).expect("translation table is valid JSON")))
}

fn table(code: &str) -> Option<&'static HashMap<String, String>> {
    let index = CODES.iter().position(|c| *c == code)?;
    index.checked_sub(1).map(|i| &tables()[i])
}

/// "" follows Windows' display language; "en", "nl", "de" or "fr" force one. Anything else is English.
pub fn use_language(setting: &str) {
    let code = if setting.is_empty() { windows_language() } else { setting.to_lowercase() };
    CURRENT.store(CODES.iter().position(|c| *c == code).unwrap_or(0), Ordering::Relaxed);
}

/// "en", "nl", "de" or "fr".
pub fn language() -> &'static str {
    CODES[CURRENT.load(Ordering::Relaxed)]
}

/// True when the text is shown in another language than English.
pub fn is_translated() -> bool {
    CURRENT.load(Ordering::Relaxed) != 0
}

/// The text in the current language.
pub fn t(english: &str) -> &str {
    find(english).unwrap_or(english)
}

/// A text with placeholders (`{0}`, `{1}`…) in the current language.
pub fn f(english: &str, args: &[&dyn Display]) -> String {
    fill(t(english), args)
}

/// The translation, or `None` when the current language is English or has no entry for the text.
pub fn find(english: &str) -> Option<&'static str> {
    table(language())?.get(english).map(String::as_str)
}

/// "1st", "1e", "1.", "1re": for "every 4th break".
pub fn ordinal(n: i64) -> String {
    match language() {
        "nl" => format!("{n}e"),
        "de" => format!("{n}."),
        "fr" if n == 1 => "1re".into(),
        "fr" => format!("{n}e"),
        _ if matches!(n % 100, 11..=13) => format!("{n}th"),
        _ => match n % 10 {
            1 => format!("{n}st"),
            2 => format!("{n}nd"),
            3 => format!("{n}rd"),
            _ => format!("{n}th"),
        },
    }
}

/// The English texts translated into one language (for the coverage tests).
pub fn known_in(language: &str) -> Vec<&'static str> {
    table(language).map(|t| t.keys().map(String::as_str).collect()).unwrap_or_default()
}

/// Replaces `{n}` / `{n:format}` with the n-th argument; `{{` and `}}` are literal braces.
pub fn fill(template: &str, args: &[&dyn Display]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(i) = rest.find(['{', '}']) {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            out.push_str(&tail[..1]);
            rest = &tail[2..];
            continue;
        }
        if let Some(end) = tail.find('}').filter(|_| tail.starts_with('{')) {
            let inner = &tail[1..end];
            let index = inner.split(':').next().unwrap_or("");
            if let Some(arg) = index.parse::<usize>().ok().and_then(|n| args.get(n)) {
                out.push_str(&arg.to_string());
                rest = &tail[end + 1..];
                continue;
            }
        }
        out.push_str(&tail[..1]); // a lone brace or a placeholder without an argument stays as written
        rest = &tail[1..];
    }
    out.push_str(rest);
    out
}

/// Windows' display language as "nl", "de", "fr" or "en".
fn windows_language() -> String {
    let id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() } & 0x3FF; // primary language
    match id {
        0x13 => "nl",
        0x07 => "de",
        0x0C => "fr",
        _ => "en",
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_placeholders() {
        assert_eq!(fill("In {0} min", &[&5]), "In 5 min");
        assert_eq!(fill("Next: {0} at {1:t}", &[&"Stand-up", &"9:00"]), "Next: Stand-up at 9:00");
        assert_eq!(fill("{1} before {0}", &[&"a", &"b"]), "b before a");
        assert_eq!(fill("{{literal}} {0}", &[&1]), "{literal} 1");
        assert_eq!(fill("missing {3}", &[&1]), "missing {3}");
    }

    #[test]
    fn english_is_untranslated() {
        // Unit tests never switch the language (that's global); tests/translations.rs runs in its own process.
        assert_eq!(t("A text without translation"), "A text without translation");
        assert_eq!(ordinal(22), "22nd");
        assert_eq!(ordinal(11), "11th");
        assert_eq!(ordinal(3), "3rd");
    }

    #[test]
    fn tables_load() {
        assert!(known_in("nl").len() > 250);
        assert!(known_in("en").is_empty());
    }
}
