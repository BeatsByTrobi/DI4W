//! The text for a GitHub issue: versions, set-up, add-ons with problems and the end of today's log.
//! The user sees and can edit all of it before anything leaves the PC, and personal paths are replaced.

use crate::system;
use di4w_core::{displays, paths, updates};
use regex::Regex;

pub fn new_issue_url() -> String {
    format!("https://github.com/{}/issues/new", updates::REPOSITORY)
}

/// GitHub (and browsers) cut off very long links; longer reports go through the clipboard.
pub const MAX_BODY_IN_LINK: usize = 6000;

/// What the report says about where it came from.
pub struct Facts {
    pub install_type: &'static str,
    pub monitor: String,
    pub island_running: bool,
}

/// `crash`: the island's last crash (crash.txt), included when reporting one.
pub fn build(crash: Option<&str>, facts: &Facts) -> String {
    let mut s = String::new();
    s.push_str("**What happened?**\n");
    s.push_str("<!-- Describe what you did, what you expected, and what happened instead. -->\n\n\n");
    if let Some(crash) = crash {
        s.push_str("**Crash**\n```\n");
        if crash.chars().count() > 3000 {
            s.push_str(&crash.chars().take(3000).collect::<String>());
            s.push_str("\n…");
        } else {
            s.push_str(crash);
        }
        s.push_str("\n```\n\n");
    }
    s.push_str("**System**\n");
    s.push_str(&format!("- Dynamic Island {} ({})\n", updates::current_version(), facts.install_type));
    s.push_str(&format!("- {}, {}\n", system::windows_version(), updates::architecture()));
    s.push_str(&format!("- Displays: {}, island on: {}\n", displays::all().len(), describe_monitor(&facts.monitor)));
    s.push_str(&format!("- Island running: {}\n", if facts.island_running { "yes" } else { "no" }));

    let problems: Vec<(String, String)> = if facts.island_running {
        di4w_core::status::IslandStatus::try_load()
            .map(|status| {
                status
                    .addons
                    .into_iter()
                    .filter_map(|(key, state)| state.message.filter(|_| state.problem).map(|m| (key, m)))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    if !problems.is_empty() {
        s.push_str("\n**Add-ons reporting a problem**\n");
        for (key, message) in problems {
            s.push_str(&format!("- {key}: {message}\n"));
        }
    }

    let log = recent_log(40);
    if !log.is_empty() {
        s.push_str("\n**Recent log**\n```\n");
        s.push_str(&log);
        s.push_str("\n```\n");
    }
    redact(&s)
}

/// In English: the issue is read by the developer. "Crash: thread 'main' panicked at …"
pub fn crash_title(crash: &str) -> String {
    let line = crash.lines().nth(1).unwrap_or_default();
    format!("Crash: {}", line.split(':').next().unwrap_or_default().trim())
}

/// The link that opens a new issue with this title and text. A text too long for a link has been copied to the
/// clipboard instead; the issue then asks to paste it.
pub fn issue_link(title: &str, body: &str) -> String {
    let text = if body.encode_utf16().count() > MAX_BODY_IN_LINK {
        "<!-- The report was too long for a link and has been copied: paste it here (Ctrl+V). -->"
    } else {
        body
    };
    format!("{}?title={}&body={}", new_issue_url(), escape(title.trim()), escape(text))
}

/// Percent-encodes everything but unreserved characters (like Uri.EscapeDataString).
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn describe_monitor(setting: &str) -> &str {
    match setting {
        "" => "main display",
        displays::FOLLOW_MOUSE => "display with the mouse",
        other => other,
    }
}

/// The last lines of today's log.
fn recent_log(lines: usize) -> String {
    let file = paths::log_folder().join(format!("{}.log", chrono::Local::now().format("%Y-%m-%d")));
    let Ok(bytes) = std::fs::read(file) else { return String::new() };
    let text = String::from_utf8_lossy(&bytes);
    let all: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).filter(|l| !l.is_empty()).collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// Replaces the user folder, user name and e-mail addresses, so the report doesn't identify anyone.
pub fn redact(text: &str) -> String {
    let profile = std::env::var("USERPROFILE").unwrap_or_default();
    let temp = std::env::temp_dir().display().to_string();
    // The 8.3 form, e.g. C:\Users\OPGERU~1, as some paths show it.
    let short_profile = temp.split(r"\AppData\").next().unwrap_or_default().to_string();
    let user = std::env::var("USERNAME").unwrap_or_default();
    redact_with(text, &profile, &short_profile, &user)
}

fn redact_with(text: &str, profile: &str, short_profile: &str, user: &str) -> String {
    let mut text = text.to_string();
    let replace = |text: &str, what: &str, with: &str| -> String {
        Regex::new(&format!("(?i){}", regex::escape(what))).map(|r| r.replace_all(text, with).into_owned()).unwrap_or_else(|_| text.into())
    };
    if profile.len() > 3 {
        text = replace(&text, profile, "%USERPROFILE%");
    }
    if short_profile.len() > 3 && !short_profile.eq_ignore_ascii_case(profile) {
        text = replace(&text, short_profile, "%USERPROFILE%");
    }
    if user.chars().count() > 2 {
        text = replace(&text, user, "<user>");
    }
    let email = Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").unwrap();
    email.replace_all(&text, "<email>").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_personal_details() {
        let text = r"Loaded C:\Users\Jane.Doe\AppData\Roaming\x and c:\users\JANE~1\Temp, user jane.doe, mail jane@example.com";
        let out = redact_with(text, r"C:\Users\Jane.Doe", r"C:\Users\JANE~1", "Jane.Doe");
        assert_eq!(out, r"Loaded %USERPROFILE%\AppData\Roaming\x and %USERPROFILE%\Temp, user <user>, mail <email>");
    }

    #[test]
    fn escapes_like_uri_escape_data_string() {
        assert_eq!(escape("Weather doesn't show & more"), "Weather%20doesn%27t%20show%20%26%20more");
        assert_eq!(escape("a-b_c.d~é"), "a-b_c.d~%C3%A9");
    }

    #[test]
    fn long_reports_go_through_the_clipboard() {
        let link = issue_link("Title", &"x".repeat(MAX_BODY_IN_LINK + 1));
        assert!(link.contains("too%20long%20for%20a%20link"));
        assert!(issue_link("T", "short").ends_with("&body=short"));
    }

    #[test]
    fn crash_titles_name_the_error() {
        assert_eq!(crash_title("2026-11-18 10:00:00 (version 2.0.0)\nthread 'main' panicked at src\\x.rs:1\nmore"), "Crash: thread 'main' panicked at src\\x.rs");
    }
}
