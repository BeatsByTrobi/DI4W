//! A small iCalendar (.ics) reader for meeting reminders: events with their recurrences (RRULE, EXDATE, moved
//! occurrences via RECURRENCE-ID) and time zones (the file's own VTIMEZONE definitions, IANA names, UTC, or floating
//! local time). All-day and cancelled events are skipped.

use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use rrule::{RRule, Tz, Unvalidated};
use std::collections::{HashMap, HashSet};

/// A meeting occurrence in the window.
#[derive(Clone, Debug, PartialEq)]
pub struct Meeting {
    /// Stable per occurrence (UID and start), so each is announced once.
    pub key: String,
    pub title: Option<String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub location: Option<String>,
    pub description: String,
    pub url: String,
}

#[derive(Clone, Debug, Default)]
struct Property {
    name: String,
    params: HashMap<String, String>,
    value: String,
}

#[derive(Clone, Debug, Default)]
struct Component {
    name: String,
    props: Vec<Property>,
    children: Vec<Component>,
}

impl Component {
    fn prop(&self, name: &str) -> Option<&Property> {
        self.props.iter().find(|p| p.name == name)
    }

    fn value(&self, name: &str) -> Option<String> {
        self.prop(name).map(|p| unescape(&p.value))
    }
}

fn unescape(text: &str) -> String {
    text.replace("\\n", "\n").replace("\\N", "\n").replace("\\,", ",").replace("\\;", ";").replace("\\\\", "\\")
}

/// Lines with their folds undone ("a line that goes on\r\n  here").
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if (line.starts_with(' ') || line.starts_with('\t')) && !lines.is_empty() {
            lines.last_mut().unwrap().push_str(&line[1..]);
        } else {
            lines.push(line.to_string());
        }
    }
    lines
}

fn parse_property(line: &str) -> Option<Property> {
    // NAME;PARAM=value;PARAM="quoted":VALUE — the first colon outside quotes ends the name and parameters.
    let mut in_quotes = false;
    let mut colon = None;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let mut parts = head.split(';');
    let name = parts.next()?.trim().to_uppercase();
    let params = parts
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((k.trim().to_uppercase(), v.trim().trim_matches('"').to_string()))
        })
        .collect();
    Some(Property { name, params, value: value.to_string() })
}

fn parse_components(text: &str) -> Vec<Component> {
    let mut stack: Vec<Component> = vec![Component::default()];
    for line in unfold(text) {
        let Some(p) = parse_property(&line) else { continue };
        match p.name.as_str() {
            "BEGIN" => stack.push(Component { name: p.value.trim().to_uppercase(), ..Default::default() }),
            "END" => {
                if stack.len() > 1 {
                    let done = stack.pop().unwrap();
                    stack.last_mut().unwrap().children.push(done);
                }
            }
            _ => stack.last_mut().unwrap().props.push(p),
        }
    }
    while stack.len() > 1 {
        let done = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(done);
    }
    stack.pop().map(|root| root.children).unwrap_or_default()
}

/// Where a date-time's clock reading is.
#[derive(Clone, Debug, PartialEq)]
enum Zone {
    Utc,
    /// A TZID (resolved through the file's VTIMEZONE, or as an IANA name).
    Named(String),
    /// No zone: the PC's local time.
    Floating,
}

/// A date-time as written: the wall-clock reading and its zone, or a whole day.
#[derive(Clone, Debug)]
struct When {
    wall: NaiveDateTime,
    zone: Zone,
    all_day: bool,
}

fn parse_when(p: &Property) -> Option<When> {
    let v = p.value.trim();
    let all_day = p.params.get("VALUE").is_some_and(|x| x.eq_ignore_ascii_case("DATE")) || (v.len() == 8 && !v.contains('T'));
    if all_day {
        let d = NaiveDate::parse_from_str(&v[..8.min(v.len())], "%Y%m%d").ok()?;
        return Some(When { wall: d.and_time(NaiveTime::MIN), zone: Zone::Floating, all_day: true });
    }
    let (text, utc) = match v.strip_suffix('Z') {
        Some(t) => (t, true),
        None => (v, false),
    };
    let wall = NaiveDateTime::parse_from_str(text, "%Y%m%dT%H%M%S").ok()?;
    let zone = if utc {
        Zone::Utc
    } else if let Some(tz) = p.params.get("TZID") {
        Zone::Named(tz.clone())
    } else {
        Zone::Floating
    };
    Some(When { wall, zone, all_day: false })
}

/// "PT15M", "P1DT2H", "P1W", "-PT5M".
fn parse_duration(text: &str) -> Option<Duration> {
    let t = text.trim();
    let (negative, t) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let t = t.strip_prefix('P')?;
    let mut total = Duration::zero();
    let mut number = String::new();
    for c in t.chars() {
        match c {
            '0'..='9' => number.push(c),
            'T' => {}
            'W' | 'D' | 'H' | 'M' | 'S' => {
                let n: i64 = number.parse().ok()?;
                number.clear();
                total += match c {
                    'W' => Duration::weeks(n),
                    'D' => Duration::days(n),
                    'H' => Duration::hours(n),
                    'M' => Duration::minutes(n),
                    _ => Duration::seconds(n),
                };
            }
            _ => return None,
        }
    }
    Some(if negative { -total } else { total })
}

/// A VTIMEZONE: its standard/daylight rules (local onset, offset before, offset after, recurrence).
struct ZoneRules {
    onsets: Vec<(NaiveDateTime, i32, i32, Option<String>)>,
}

fn offset(text: &str) -> Option<i32> {
    let t = text.trim();
    let sign = if t.starts_with('-') { -1 } else { 1 };
    let digits: String = t.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 4 {
        return None;
    }
    let h: i32 = digits[0..2].parse().ok()?;
    let m: i32 = digits[2..4].parse().ok()?;
    let s: i32 = digits.get(4..6).and_then(|s| s.parse().ok()).unwrap_or(0);
    Some(sign * (h * 3600 + m * 60 + s))
}

impl ZoneRules {
    fn from(c: &Component) -> ZoneRules {
        let onsets = c
            .children
            .iter()
            .filter(|s| s.name == "STANDARD" || s.name == "DAYLIGHT")
            .filter_map(|s| {
                let start = parse_when(s.prop("DTSTART")?)?.wall;
                Some((start, offset(&s.value("TZOFFSETFROM")?)?, offset(&s.value("TZOFFSETTO")?)?, s.value("RRULE")))
            })
            .collect();
        ZoneRules { onsets }
    }

    /// The UTC offset (seconds) in effect at a wall-clock time.
    fn offset_at(&self, wall: NaiveDateTime) -> Option<i32> {
        let mut latest: Option<(NaiveDateTime, i32)> = None;
        for (start, _, to, rule) in &self.onsets {
            let candidates: Vec<NaiveDateTime> = match rule {
                Some(r) if *start <= wall => expand(r, *start, wall - Duration::days(400), wall + Duration::seconds(1)),
                _ => vec![*start],
            };
            for c in candidates.into_iter().filter(|c| *c <= wall) {
                if latest.is_none_or(|(l, _)| c > l) {
                    latest = Some((c, *to));
                }
            }
        }
        latest.map(|(_, o)| o).or_else(|| self.onsets.iter().min_by_key(|o| o.0).map(|o| o.1))
    }
}

fn fake_utc(wall: NaiveDateTime) -> DateTime<Tz> {
    Tz::UTC.from_utc_datetime(&wall)
}

/// Recurrences of a rule in wall-clock time (the rule is expanded as if the clock were UTC, so "every Monday at
/// 9:00" stays at 9:00 across daylight saving changes).
fn expand(rule: &str, start: NaiveDateTime, from: NaiveDateTime, to: NaiveDateTime) -> Vec<NaiveDateTime> {
    // UNTIL is set separately, as a wall-clock time like the start.
    let mut until = None;
    let parts: Vec<&str> = rule
        .split(';')
        .filter(|p| match p.split_once('=') {
            Some((k, v)) if k.trim().eq_ignore_ascii_case("UNTIL") => {
                let v = v.trim();
                until = NaiveDateTime::parse_from_str(v.trim_end_matches('Z'), "%Y%m%dT%H%M%S")
                    .ok()
                    .or_else(|| NaiveDate::parse_from_str(&v[..8.min(v.len())], "%Y%m%d").ok().map(|d| d.and_hms_opt(23, 59, 59).unwrap()));
                false
            }
            _ => !p.trim().is_empty(),
        })
        .collect();
    let Ok(mut parsed) = parts.join(";").parse::<RRule<Unvalidated>>() else { return vec![start] };
    if let Some(u) = until {
        if u < start {
            return vec![start];
        }
        parsed = parsed.until(fake_utc(u));
    }
    let Ok(set) = parsed.build(fake_utc(start)) else { return vec![start] };
    set.after(fake_utc(from)).before(fake_utc(to)).all(1000).dates.into_iter().map(|d| d.naive_utc()).collect()
}

struct Resolver {
    zones: HashMap<String, ZoneRules>,
}

impl Resolver {
    fn to_utc(&self, wall: NaiveDateTime, zone: &Zone) -> DateTime<Utc> {
        match zone {
            Zone::Utc => Utc.from_utc_datetime(&wall),
            Zone::Named(id) => {
                if let Some(off) = self.zones.get(id).and_then(|z| z.offset_at(wall)) {
                    return Utc.from_utc_datetime(&(wall - Duration::seconds(off as i64)));
                }
                if let Ok(tz) = id.parse::<chrono_tz::Tz>()
                    && let Some(t) = tz.from_local_datetime(&wall).earliest() {
                        return t.with_timezone(&Utc);
                    }
                Self::local(wall)
            }
            Zone::Floating => Self::local(wall),
        }
    }

    fn local(wall: NaiveDateTime) -> DateTime<Utc> {
        Local.from_local_datetime(&wall).earliest().map(|t| t.with_timezone(&Utc)).unwrap_or_else(|| Utc.from_utc_datetime(&wall))
    }

    /// A UTC instant's wall-clock reading in a zone (for the expansion window).
    fn to_wall(&self, utc: DateTime<Utc>, zone: &Zone) -> NaiveDateTime {
        let guess = utc.naive_utc();
        let back = self.to_utc(guess, zone);
        guess + (utc - back)
    }
}

/// Occurrences of the events that overlap `[from, to)`.
pub fn parse(ics: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<Meeting> {
    let calendars = parse_components(ics);
    let all: Vec<&Component> = calendars.iter().flat_map(|c| if c.name == "VCALENDAR" { c.children.iter().collect::<Vec<_>>() } else { vec![c] }).collect();
    let resolver = Resolver {
        zones: all.iter().filter(|c| c.name == "VTIMEZONE").filter_map(|c| Some((c.value("TZID")?, ZoneRules::from(c)))).collect(),
    };
    let events: Vec<&Component> = all.iter().copied().filter(|c| c.name == "VEVENT").collect();

    // Moved or changed occurrences of recurring events.
    let mut overridden: HashSet<(String, i64)> = HashSet::new();
    for e in &events {
        if let (Some(uid), Some(rid)) = (e.value("UID"), e.prop("RECURRENCE-ID").and_then(parse_when)) {
            overridden.insert((uid, resolver.to_utc(rid.wall, &rid.zone).timestamp()));
        }
    }

    let mut result = Vec::new();
    for e in events {
        let Some(start) = e.prop("DTSTART").and_then(parse_when) else { continue };
        if start.all_day || e.value("STATUS").is_some_and(|s| s.trim().eq_ignore_ascii_case("CANCELLED")) {
            continue;
        }
        let uid = e.value("UID").unwrap_or_default();
        let length = match (e.prop("DTEND").and_then(parse_when), e.value("DURATION").and_then(|d| parse_duration(&d))) {
            (Some(end), _) => resolver.to_utc(end.wall, &end.zone) - resolver.to_utc(start.wall, &start.zone),
            (None, Some(d)) => d,
            (None, None) => Duration::minutes(30),
        };
        let is_override = e.prop("RECURRENCE-ID").is_some();
        let walls = match e.value("RRULE") {
            Some(rule) if !is_override => {
                let lo = resolver.to_wall(from - length - Duration::days(1), &start.zone);
                let hi = resolver.to_wall(to + Duration::days(1), &start.zone);
                let excluded: HashSet<i64> = e
                    .props
                    .iter()
                    .filter(|p| p.name == "EXDATE")
                    .flat_map(|p| p.value.split(',').filter_map(|v| parse_when(&Property { value: v.to_string(), ..p.clone() })).collect::<Vec<_>>())
                    .map(|w| resolver.to_utc(w.wall, &w.zone).timestamp())
                    .collect();
                expand(&rule, start.wall, lo, hi)
                    .into_iter()
                    .filter(|w| {
                        let t = resolver.to_utc(*w, &start.zone).timestamp();
                        !excluded.contains(&t) && !overridden.contains(&(uid.clone(), t))
                    })
                    .collect()
            }
            _ => vec![start.wall],
        };
        for wall in walls {
            let begin = resolver.to_utc(wall, &start.zone);
            let end = begin + length;
            if begin >= to || end <= from {
                continue;
            }
            result.push(Meeting {
                key: format!("{uid}|{}", begin.to_rfc3339()),
                title: e.value("SUMMARY"),
                start: begin,
                end,
                location: e.value("LOCATION"),
                description: e.value("DESCRIPTION").unwrap_or_default(),
                url: e.value("URL").unwrap_or_default(),
            });
        }
    }
    result.sort_by_key(|m| m.start);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_durations() {
        assert_eq!(parse_duration("PT15M"), Some(Duration::minutes(15)));
        assert_eq!(parse_duration("P1DT2H"), Some(Duration::hours(26)));
        assert_eq!(parse_duration("P1W"), Some(Duration::weeks(1)));
        assert_eq!(parse_duration("nope"), None);
    }

    #[test]
    fn follows_the_files_time_zone() {
        // Outlook writes Windows zone names with a VTIMEZONE; 09:00 in Amsterdam is 07:00 UTC in summer, 08:00 in winter.
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VTIMEZONE\r\nTZID:W. Europe Standard Time\r\nBEGIN:STANDARD\r\nDTSTART:16010101T030000\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\nRRULE:FREQ=YEARLY;BYDAY=-1SU;BYMONTH=10\r\nEND:STANDARD\r\nBEGIN:DAYLIGHT\r\nDTSTART:16010101T020000\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0200\r\nRRULE:FREQ=YEARLY;BYDAY=-1SU;BYMONTH=3\r\nEND:DAYLIGHT\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:x\r\nDTSTART;TZID=W. Europe Standard Time:20260601T090000\r\nDTEND;TZID=W. Europe Standard Time:20260601T093000\r\nRRULE:FREQ=WEEKLY;COUNT=40\r\nSUMMARY:Weekly\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let june = parse(ics, Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap(), Utc.with_ymd_and_hms(2026, 6, 2, 0, 0, 0).unwrap());
        assert_eq!(june[0].start, Utc.with_ymd_and_hms(2026, 6, 1, 7, 0, 0).unwrap());
        let december = parse(ics, Utc.with_ymd_and_hms(2026, 11, 30, 0, 0, 0).unwrap(), Utc.with_ymd_and_hms(2026, 12, 1, 0, 0, 0).unwrap());
        assert_eq!(december[0].start, Utc.with_ymd_and_hms(2026, 11, 30, 8, 0, 0).unwrap());
    }

    #[test]
    fn skips_excluded_and_moved_occurrences() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:d\nDTSTART:20261005T100000Z\nDURATION:PT30M\nRRULE:FREQ=DAILY;UNTIL=20261010T100000Z\nEXDATE:20261006T100000Z\nSUMMARY:Daily\nEND:VEVENT\nBEGIN:VEVENT\nUID:d\nRECURRENCE-ID:20261007T100000Z\nDTSTART:20261007T150000Z\nDURATION:PT30M\nSUMMARY:Daily (moved)\nEND:VEVENT\nEND:VCALENDAR\n";
        let all = parse(ics, Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap(), Utc.with_ymd_and_hms(2026, 10, 20, 0, 0, 0).unwrap());
        let starts: Vec<String> = all.iter().map(|m| m.start.format("%d %H").to_string()).collect();
        assert_eq!(starts, vec!["05 10", "07 15", "08 10", "09 10", "10 10"]);
    }
}
