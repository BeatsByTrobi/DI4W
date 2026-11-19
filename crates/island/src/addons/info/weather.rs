//! Current temperature and an icon in the empty pill, plus "Rain in about 15 min" warnings. Uses Open-Meteo (free,
//! no account or key). Location: the city from the settings, else Windows location.

use crate::addons::{Addon, Base, Context};
use crate::model::Alert;
use crate::rt::Timer;
use crate::ui::colors::{self, Color};
use crate::ui::el::{Align, El};
use crate::ui::{self, ICON_SIZE, SEMIBOLD};
use di4w_core::{AddonSettings, l};
use serde_json::Value;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};
use windows::Devices::Geolocation::{GeolocationAccessStatus, Geolocator, PositionAccuracy};
use windows::Foundation::TimeSpan;

#[derive(Clone, Debug)]
struct Place {
    query: String,
    lat: f64,
    lon: f64,
    name: String,
}

/// What a refresh found (worked out on a background thread).
enum Outcome {
    Weather { place: Place, temp: f64, code: i64, day: bool, rain_in: Option<i64> },
    /// No location: the status message to show.
    NoPlace(String),
    Unreachable,
    Failed(String),
}

#[derive(Default)]
struct Shown {
    symbol: String,
    color: Option<Color>,
    temp: String,
    tooltip: String,
}

pub struct Weather {
    base: Base,
    shown: Rc<RefCell<Shown>>,
    view: Rc<dyn Fn() -> El>,
    refresh: RefCell<Option<Timer>>,
    location: RefCell<Option<Place>>,
    last_rain_alert: RefCell<Option<Instant>>,
    options_key: RefCell<String>,
}

fn is_wet(code: i64) -> bool {
    (51..=67).contains(&code) || (80..=82).contains(&code) || code >= 95
}

/// WMO weather code -> symbol, color and words.
fn describe(code: i64, day: bool) -> (&'static str, Color, &'static str) {
    match code {
        0 if day => ("☀", colors::amber(), l::t("Clear sky")),
        0 => ("☾", colors::teal(), l::t("Clear sky")),
        1 | 2 if day => ("⛅", colors::amber(), l::t("Partly cloudy")),
        1 | 2 => ("☁", colors::grey(), l::t("Partly cloudy")),
        3 => ("☁", colors::grey(), l::t("Cloudy")),
        45 | 48 => ("☁", colors::grey(), l::t("Fog")),
        51..=67 | 80..=82 => ("☂", colors::blue(), l::t("Rain")),
        71..=77 | 85 | 86 => ("❄", colors::primary(), l::t("Snow")),
        c if c >= 95 => ("⛈", colors::amber(), l::t("Thunderstorm")),
        _ => ("☁", colors::grey(), l::t("Cloudy")),
    }
}

fn get_json(url: &str) -> Result<Value, ureq::Error> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).build().into();
    let text = agent.get(url).header("User-Agent", di4w_core::updates::USER_AGENT).call()?.body_mut().read_to_string()?;
    serde_json::from_str(&text).map_err(|e| ureq::Error::Io(std::io::Error::other(e)))
}

fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Finds the place (cached while the city stays the same) and reads the weather. Blocks.
fn fetch(query: String, cached: Option<Place>, fahrenheit: bool, rain_alerts: bool) -> Outcome {
    let place = match cached.filter(|c| c.query == query) {
        Some(p) => p,
        None if !query.is_empty() => {
            let url = format!("https://geocoding-api.open-meteo.com/v1/search?count=1&name={}", encode(&query));
            match get_json(&url) {
                Ok(json) => match json.get("results").and_then(Value::as_array).and_then(|r| r.first()) {
                    Some(r) => Place {
                        query: query.clone(),
                        lat: r.get("latitude").and_then(Value::as_f64).unwrap_or(0.0),
                        lon: r.get("longitude").and_then(Value::as_f64).unwrap_or(0.0),
                        name: r.get("name").and_then(Value::as_str).unwrap_or(&query).to_string(),
                    },
                    None => {
                        di4w_core::log::info(format!("Weather: place '{query}' not found"));
                        return Outcome::NoPlace(l::f("Couldn't find \"{0}\". Check the spelling, or try a bigger city nearby.", &[&query]));
                    }
                },
                Err(e) => return error(e),
            }
        }
        None => {
            // No city set: ask Windows (works when location is on for desktop apps).
            let allowed = Geolocator::RequestAccessAsync().and_then(|o| o.join()).is_ok_and(|s| s == GeolocationAccessStatus::Allowed);
            if !allowed {
                di4w_core::log::info("Weather: no city set and Windows location is off");
                return Outcome::NoPlace(l::t("Set your city below. (Windows location is off for desktop apps.)").into());
            }
            let position = (|| -> windows::core::Result<(f64, f64)> {
                let locator = Geolocator::new()?;
                locator.SetDesiredAccuracy(PositionAccuracy::Default)?;
                let pos = locator.GetGeopositionAsyncWithAgeAndTimeout(TimeSpan { Duration: 36_000_000_000 }, TimeSpan { Duration: 100_000_000 })?.join()?;
                let p = pos.Coordinate()?.Point()?.Position()?;
                Ok((p.Latitude, p.Longitude))
            })();
            match position {
                Ok((lat, lon)) => Place { query: String::new(), lat, lon, name: l::t("your area").into() },
                Err(e) => return Outcome::Failed(e.message()),
            }
        }
    };

    let unit = if fahrenheit { "fahrenheit" } else { "celsius" };
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&current=temperature_2m,weather_code,is_day&minutely_15=precipitation&forecast_minutely_15=4&temperature_unit={unit}&timezone=auto",
        place.lat, place.lon
    );
    let json = match get_json(&url) {
        Ok(j) => j,
        Err(e) => return error(e),
    };
    let Some(current) = json.get("current") else { return Outcome::Failed("no current weather".into()) };
    let temp = current.get("temperature_2m").and_then(Value::as_f64).unwrap_or(0.0);
    let code = current.get("weather_code").and_then(Value::as_i64).unwrap_or(3);
    let day = current.get("is_day").and_then(Value::as_i64) == Some(1);

    // Rain in the next 15–45 minutes while it's dry now.
    let rain_in = (rain_alerts && !is_wet(code))
        .then(|| {
            let rain: Vec<f64> = json.get("minutely_15")?.get("precipitation")?.as_array()?.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect();
            if rain.len() < 3 || rain[0] > 0.05 {
                return None;
            }
            let first = rain.iter().skip(1).take(3).position(|r| *r >= 0.1)?;
            Some((first as i64 + 1) * 15)
        })
        .flatten();
    Outcome::Weather { place, temp, code, day, rain_in }
}

fn error(e: ureq::Error) -> Outcome {
    di4w_core::log::info(format!("Weather update failed: {e}"));
    match e {
        ureq::Error::StatusCode(_) | ureq::Error::Io(_) | ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Timeout(_) => Outcome::Unreachable,
        other => Outcome::Failed(other.to_string()),
    }
}

impl Weather {
    pub fn new(ctx: &Rc<Context>) -> Rc<Weather> {
        let shown = Rc::new(RefCell::new(Shown::default()));
        let s = shown.clone();
        let view: Rc<dyn Fn() -> El> = Rc::new(move || {
            let s = s.borrow();
            El::hstack(vec![ui::symbol(&s.symbol, 14.0, s.color.unwrap_or_else(colors::grey)), ui::digits(s.temp.as_str(), 14.0, SEMIBOLD, colors::primary()).margin(5.0, 0.0, 0.0, 0.0)])
                .tooltip(s.tooltip.clone())
        });
        Rc::new(Weather {
            base: Base::new(ctx, "Weather", "Weather"),
            shown,
            view,
            refresh: RefCell::new(None),
            location: RefCell::new(None),
            last_rain_alert: RefCell::new(None),
            options_key: RefCell::new(String::new()),
        })
    }

    fn key(&self) -> String {
        let o = self.base.options();
        format!("{}|{}", o.weather_location.trim(), o.weather_fahrenheit)
    }

    fn refresh(self: &Rc<Self>) {
        let o = self.base.options();
        let (query, fahrenheit, rain) = (o.weather_location.trim().to_string(), o.weather_fahrenheit, o.rain_alerts);
        let cached = self.location.borrow().clone();
        let me = Rc::downgrade(self);
        crate::rt::spawn(move || fetch(query, cached, fahrenheit, rain), move |outcome| {
            if let Some(w) = me.upgrade() {
                w.apply(outcome);
            }
        });
    }

    fn apply(&self, outcome: Outcome) {
        if !self.base.running() {
            return;
        }
        match outcome {
            Outcome::Weather { place, temp, code, day, rain_in } => {
                let (symbol, color, words) = describe(code, day);
                let rounded = temp.round_ties_even();
                let text = l::f("{0}, {1}° in {2}", &[&words, &rounded, &place.name]);
                *self.shown.borrow_mut() = Shown { symbol: symbol.into(), color: Some(color), temp: format!("{rounded}°"), tooltip: text.clone() };
                self.base.ctx.set_idle_trailing(Some(self.view.clone()));
                crate::activities::redraw();
                self.base.set_status(Some(text), false);
                if let Some(minutes) = rain_in
                    && self.last_rain_alert.borrow().is_none_or(|t| t.elapsed() >= Duration::from_secs(2 * 3600)) {
                        *self.last_rain_alert.borrow_mut() = Some(Instant::now());
                        self.rain_alert(minutes, place.name.clone());
                    }
                *self.location.borrow_mut() = Some(place);
            }
            Outcome::NoPlace(message) => {
                self.base.ctx.set_idle_trailing(None);
                self.base.set_status(Some(message), true);
            }
            Outcome::Unreachable => self.base.set_status(Some(l::t("Couldn't reach the weather service. Trying again in 15 minutes.").into()), true),
            Outcome::Failed(e) => self.base.set_status(Some(l::f("Weather update failed: {0}", &[&e])), true),
        }
    }

    fn rain_alert(&self, minutes: i64, place: String) {
        let alert = Alert::new(move || {
            let icon = El::boxed(vec![ui::symbol("☂", 22.0, colors::on_color()).align(Align::Center, Align::Center)]).size(ICON_SIZE, ICON_SIZE).radius(ICON_SIZE / 2.0).bg(colors::blue());
            ui::alert_view(icon, &l::f("Rain in about {0} min", &[&minutes]), &place, None, None)
        })
        .seconds(6.0);
        self.base.show(alert.rc());
    }
}

impl Addon for Weather {
    fn base(&self) -> &Base {
        &self.base
    }

    fn enabled(&self, o: &AddonSettings) -> bool {
        o.weather
    }

    fn start(self: Rc<Self>) -> Result<(), String> {
        *self.options_key.borrow_mut() = self.key();
        let weak = Rc::downgrade(&self);
        *self.refresh.borrow_mut() = Some(Timer::every(Duration::from_secs(15 * 60), move || {
            if let Some(w) = weak.upgrade() {
                w.refresh();
            }
        }));
        self.refresh();
        Ok(())
    }

    fn stop(&self) {
        *self.refresh.borrow_mut() = None;
        self.base.ctx.set_idle_trailing(None);
    }

    // A new city or unit should show now, not at the next 15-minute refresh.
    fn options_changed(self: Rc<Self>) {
        let key = self.key();
        if *self.options_key.borrow() == key {
            return;
        }
        *self.options_key.borrow_mut() = key;
        self.refresh();
    }

    fn preview(self: Rc<Self>) {
        let city = self.base.options().weather_location.trim().to_string();
        let place = if !city.is_empty() { city } else { self.location.borrow().as_ref().map_or("Amsterdam".to_string(), |p| p.name.clone()) };
        self.rain_alert(15, place);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_weather_codes() {
        assert_eq!(describe(0, true).0, "☀");
        assert_eq!(describe(0, false).0, "☾");
        assert_eq!(describe(61, true).0, "☂");
        assert_eq!(describe(96, true).0, "⛈");
        assert!(is_wet(81) && !is_wet(3));
        assert_eq!(encode("Den Haag"), "Den%20Haag");
    }
}
