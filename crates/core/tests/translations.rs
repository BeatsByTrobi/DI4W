//! Every text the apps translate must have a version in each language, and no translation may be left over after
//! its English text is gone from the code. Translated texts are the literals passed to `l::t` / `l::f` in Rust
//! and to `L.t` / `L.f` in the settings app's `.slint` files.
//!
//! Runs in its own process (an integration test), because switching the language is global.

use di4w_core::l;
use regex::Regex;
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

const LANGUAGES: [&str; 3] = ["nl", "de", "fr"];

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

/// Every .rs and .slint file of the apps (not the translation tables, not build output).
fn source_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if name != "target" && name != "strings" && name != "tests" {
                    walk(&path, out);
                }
            } else if name.ends_with(".rs") || name.ends_with(".slint") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(&crates_dir(), &mut files);
    files
}

fn unescape(literal: &str) -> String {
    let mut out = String::new();
    let mut chars = literal.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some('u') => {
                // \u{2026}
                let hex: String = chars.by_ref().skip(1).take_while(|c| *c != '}').collect();
                if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    out.push(ch);
                }
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

const LITERAL: &str = r#""((?:[^"\\\r\n]|\\.)*)""#;

/// The texts that must be translated.
fn translated_texts() -> BTreeSet<String> {
    let call = Regex::new(&format!(r#"(?:\bl::|\bL\.)[tf]\(\s*{LITERAL}"#)).unwrap();
    let mut texts = BTreeSet::new();
    for file in source_files() {
        let code = std::fs::read_to_string(&file).unwrap();
        texts.extend(call.captures_iter(&code).map(|c| unescape(&c[1])));
    }
    texts
}

/// Every string literal in the code (labels passed around in variables count as used too).
fn all_strings() -> HashSet<String> {
    let literal = Regex::new(LITERAL).unwrap();
    let mut strings = HashSet::new();
    for file in source_files() {
        let code = std::fs::read_to_string(&file).unwrap();
        strings.extend(literal.captures_iter(&code).map(|c| unescape(&c[1])));
    }
    strings
}

#[test]
fn every_text_has_a_translation() {
    let texts = translated_texts();
    assert!(texts.len() > 250, "Only found {} texts; is the scan still looking in the right place?", texts.len());
    for language in LANGUAGES {
        let known: HashSet<&str> = l::known_in(language).into_iter().collect();
        let missing: Vec<&String> = texts.iter().filter(|t| !known.contains(t.as_str())).collect();
        assert!(missing.is_empty(), "Missing {language} translations:\n{}", missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n"));
    }
}

#[test]
fn no_translation_is_left_over() {
    let mut used = all_strings();
    used.extend(translated_texts());
    for language in LANGUAGES {
        let mut unused: Vec<&str> = l::known_in(language).into_iter().filter(|k| !used.contains(*k)).collect();
        unused.sort();
        assert!(unused.is_empty(), "{language} translations whose English text no longer appears in the code:\n{}", unused.join("\n"));
    }
}

#[test]
fn translations_keep_the_same_placeholders() {
    let placeholder = Regex::new(r"\{\d+(:[^}]*)?\}").unwrap();
    let set = |text: &str| placeholder.find_iter(text).map(|m| m.as_str().to_string()).collect::<BTreeSet<_>>();
    for language in LANGUAGES {
        l::use_language(language);
        let broken: Vec<&str> = l::known_in(language).into_iter().filter(|k| set(k) != set(l::t(k))).collect();
        assert!(broken.is_empty(), "{language} placeholders differ:\n{}", broken.join("\n"));
    }
    l::use_language("en");
}

#[test]
fn tables_have_no_duplicate_keys() {
    // serde_json keeps the last of repeated keys; catch them in the source.
    let key = Regex::new(&format!(r#"(?m)^\s*{LITERAL}\s*:"#)).unwrap();
    for language in LANGUAGES {
        let path = crates_dir().join("core").join("strings").join(format!("{language}.json"));
        let json = std::fs::read_to_string(&path).unwrap();
        let mut seen = HashSet::new();
        let duplicates: Vec<String> = key.captures_iter(&json).map(|c| c[1].to_string()).filter(|k| !seen.insert(k.clone())).collect();
        assert!(duplicates.is_empty(), "{language}.json repeats: {}", duplicates.join(", "));
    }
}

#[test]
fn formats_in_each_language() {
    l::use_language("nl");
    assert_eq!(l::f("In {0} min", &[&5]), "Over 5 min");
    assert_eq!(l::t("Low battery"), "Batterij raakt leeg");
    assert_eq!(l::t("A text without translation"), "A text without translation");
    assert_eq!(l::ordinal(4), "4e");
    l::use_language("de");
    assert_eq!(l::ordinal(4), "4.");
    l::use_language("fr");
    assert_eq!(l::ordinal(1), "1re");
    l::use_language("xx");
    assert!(!l::is_translated());
    l::use_language("en");
    assert_eq!(l::f("In {0} min", &[&5]), "In 5 min");
    assert_eq!(l::ordinal(22), "22nd");
}
