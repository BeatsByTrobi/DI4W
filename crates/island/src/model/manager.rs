//! Which activities run (and which one gets the pill), and the line of alerts waiting to pop out.
//!
//! Alerts show one at a time. An urgent one (volume, brightness) jumps ahead and the interrupted one comes back
//! after it. Do Not Disturb (`quiet`) only lets alarms and calls through; a hidden island (`suppressed`) sends
//! alerts straight to Recent. Alerts that waited too long behind others are out of date and go to Recent too.

use super::activity::{Activity, Alert};
use crate::rt::Timer;
use crate::ui;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

/// How long an alert may wait behind others (e.g. behind a sticky alert while you're away).
pub const MAX_WAIT: Duration = Duration::from_secs(30);

const HISTORY_SIZE: usize = 10;

/// An alert that was shown (or held back by Do Not Disturb), as text for the Recent list.
#[derive(Clone)]
pub struct PastAlert {
    pub time: chrono::DateTime<chrono::Local>,
    pub title: String,
    pub detail: String,
    pub on_click: Option<Rc<dyn Fn()>>,
    pub missed: bool,
    pub source: Rc<Alert>,
}

struct State {
    activities: Vec<Rc<dyn Activity>>,
    alerts: VecDeque<Rc<Alert>>,
    /// When each waiting alert joined the line; one that waited too long is out of date (see `present_next`).
    queued_at: Vec<(Rc<Alert>, Instant)>,
    current: Option<Rc<Alert>>,
    hold: bool,
    remaining: Duration,
    last_tick: Instant,
    timer: Option<Timer>,
    pinned: Option<Rc<dyn Activity>>,
    order: Vec<String>,
    history: VecDeque<PastAlert>,
    samples: Vec<Weak<Alert>>,
    recording: bool,
    quiet: bool,
    suppressed: bool,
}

pub struct Manager {
    s: RefCell<State>,
    me: Weak<Manager>,
    on_changed: RefCell<Option<Rc<dyn Fn()>>>,
    /// The time, replaceable in tests.
    clock: RefCell<Box<dyn Fn() -> Instant>>,
    /// Inside a call that will notify at the end (so one change makes one refresh).
    batching: Cell<bool>,
}

fn same(a: &Rc<Alert>, b: &Rc<Alert>) -> bool {
    Rc::ptr_eq(a, b)
}

fn same_activity(a: &Rc<dyn Activity>, b: &Rc<dyn Activity>) -> bool {
    std::ptr::addr_eq(Rc::as_ptr(a), Rc::as_ptr(b))
}

/// A ringing alarm (the timer): stays until it's stopped, even while the island is hidden.
fn is_alarm(alert: &Alert) -> bool {
    alert.sticky.get() && alert.breaks_through_quiet
}

/// Callbacks to run once the state is no longer borrowed (they may call back into the manager).
#[derive(Default)]
struct After {
    calls: Vec<Rc<dyn Fn()>>,
    changed: bool,
}

impl Manager {
    pub fn new() -> Rc<Manager> {
        Rc::new_cyclic(|me| Manager {
            s: RefCell::new(State {
                activities: Vec::new(),
                alerts: VecDeque::new(),
                queued_at: Vec::new(),
                current: None,
                hold: false,
                remaining: Duration::ZERO,
                last_tick: Instant::now(),
                timer: None,
                pinned: None,
                order: Vec::new(),
                history: VecDeque::new(),
                samples: Vec::new(),
                recording: true,
                quiet: false,
                suppressed: false,
            }),
            me: me.clone(),
            on_changed: RefCell::new(None),
            clock: RefCell::new(Box::new(Instant::now)),
            batching: Cell::new(false),
        })
    }

    /// Called whenever what the island should show changed (the window refreshes).
    pub fn set_on_changed(&self, f: impl Fn() + 'static) {
        *self.on_changed.borrow_mut() = Some(Rc::new(f));
    }

    /// Replaces the clock (tests).
    #[cfg(test)]
    pub fn set_clock(&self, clock: impl Fn() -> Instant + 'static) {
        *self.clock.borrow_mut() = Box::new(clock);
    }

    fn now(&self) -> Instant {
        (self.clock.borrow())()
    }

    /// Tells the window something changed. Activities call this when their state or views change.
    pub fn changed(&self) {
        if self.batching.get() {
            return;
        }
        let f = self.on_changed.borrow().clone();
        if let Some(f) = f {
            f();
        }
    }

    fn finish(&self, after: After) {
        for call in after.calls {
            call();
        }
        if after.changed {
            self.changed();
        }
    }

    pub fn register(&self, activity: Rc<dyn Activity>) {
        self.s.borrow_mut().activities.push(activity);
    }

    /// Active activities, most important first. A user-pinned activity jumps to the front.
    pub fn active(&self) -> Vec<Rc<dyn Activity>> {
        let mut s = self.s.borrow_mut();
        if s.pinned.as_ref().is_some_and(|p| !p.is_active()) {
            s.pinned = None;
        }
        let mut active: Vec<(i64, usize, Rc<dyn Activity>)> =
            s.activities.iter().enumerate().filter(|(_, a)| a.is_active()).map(|(i, a)| (Self::rank(&s, a), i, a.clone())).collect();
        // Highest rank first; equal ranks keep their registration order (a stable sort, like LINQ's OrderBy).
        active.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        active.into_iter().map(|(_, _, a)| a).collect()
    }

    fn rank(s: &State, a: &Rc<dyn Activity>) -> i64 {
        if s.pinned.as_ref().is_some_and(|p| same_activity(p, a)) {
            return i64::MAX;
        }
        let kind = a.kind();
        // Ordered kinds outrank everything else; kinds the list doesn't know yet keep their own priority.
        match s.order.iter().position(|k| k.eq_ignore_ascii_case(&kind)) {
            Some(index) => 100_000 - index as i64,
            None => a.priority() as i64,
        }
    }

    /// The user's order of activity kinds, most important first (Settings → General); empty = built-in.
    pub fn set_order(&self, order: &[String]) {
        {
            let mut s = self.s.borrow_mut();
            if s.order == order {
                return;
            }
            s.order = order.to_vec();
        }
        self.changed();
    }

    /// Called when the user clicks the bubble: swap it into the main pill.
    pub fn pin(&self, activity: &Rc<dyn Activity>) {
        self.s.borrow_mut().pinned = Some(activity.clone());
        self.changed();
    }

    pub fn current_alert(&self) -> Option<Rc<Alert>> {
        self.s.borrow().current.clone()
    }

    /// Set by the window while the mouse is over the island, so alerts don't vanish under the cursor.
    pub fn set_hold(&self, hold: bool) {
        self.s.borrow_mut().hold = hold;
    }

    // ───────────────────────────── history ─────────────────────────────

    /// The latest alerts, newest first.
    pub fn history(&self) -> Vec<PastAlert> {
        self.s.borrow().history.iter().cloned().collect()
    }

    /// Off while the settings app's previews run, so samples don't end up in the history.
    pub fn recording(&self) -> bool {
        self.s.borrow().recording
    }

    pub fn set_recording(&self, on: bool) {
        self.s.borrow_mut().recording = on;
    }

    /// An alert that was held back (an add-on's quiet times): only listed in Recent.
    pub fn miss(&self, alert: Rc<Alert>) {
        let mut after = After::default();
        Self::remember(&mut self.s.borrow_mut(), &alert, true);
        after.calls.extend(alert.on_dismissed.clone());
        self.finish(after);
    }

    fn remember(s: &mut State, alert: &Rc<Alert>, missed: bool) {
        // Volume and brightness bars are momentary feedback, not news (or it was shown again after an interruption).
        if alert.urgent
            || s.samples.iter().any(|w| w.upgrade().is_some_and(|a| same(&a, alert)))
            || s.history.iter().any(|p| same(&p.source, alert))
        {
            return;
        }
        // Title and subtitle; later texts are button labels ("Later", "Show in folder"). Not the ☂ in an icon.
        let texts: Vec<String> = ui::texts(&(alert.content)()).into_iter().filter(|t| t.chars().any(char::is_alphanumeric)).collect();
        if texts.is_empty() {
            return;
        }
        s.history.push_front(PastAlert {
            time: chrono::Local::now(),
            title: texts[0].clone(),
            detail: texts.get(1).cloned().unwrap_or_default(),
            on_click: alert.on_click.clone(),
            missed,
            source: alert.clone(),
        });
        s.history.truncate(HISTORY_SIZE);
    }

    // ───────────────────────────── quiet and hidden ─────────────────────────────

    /// Do Not Disturb: only alerts with `breaks_through_quiet` are shown.
    pub fn quiet(&self) -> bool {
        self.s.borrow().quiet
    }

    pub fn set_quiet(&self, quiet: bool) {
        self.s.borrow_mut().quiet = quiet;
    }

    pub fn suppressed(&self) -> bool {
        self.s.borrow().suppressed
    }

    /// The island is hidden (by the user, or for a fullscreen app). Alerts then go straight to Recent instead of
    /// waiting in line: otherwise they'd all pop up one after another, long out of date, when it shows again.
    /// Hiding also moves the current alert and the waiting ones to Recent.
    pub fn set_suppressed(&self, value: bool) {
        let mut after = After::default();
        {
            let mut s = self.s.borrow_mut();
            if s.suppressed == value {
                return;
            }
            s.suppressed = value;
            s.hold = false; // the mouse can't be over a hidden island
            if !value {
                return;
            }

            // Alarms (a finished timer) keep ringing until stopped, so they stay; the rest goes to Recent.
            let mut dropped: Vec<Rc<Alert>> = s.alerts.iter().filter(|a| !is_alarm(a)).cloned().collect();
            let kept: Vec<Rc<Alert>> = s.alerts.iter().filter(|a| is_alarm(a)).cloned().collect();
            let current = s.current.clone();
            if let Some(c) = current.as_ref().filter(|c| !is_alarm(c)) {
                dropped.insert(0, c.clone());
            }
            if dropped.is_empty() {
                return;
            }

            s.alerts.clear();
            s.queued_at.clear();
            s.alerts.extend(kept);
            if current.as_ref().is_some_and(|c| !is_alarm(c)) {
                s.current = None;
                s.timer = None;
            }
            for alert in &dropped {
                Self::remember(&mut s, alert, true);
                after.calls.extend(alert.on_dismissed.clone());
            }
        }
        if self.s.borrow().current.is_none() {
            self.present_next(&mut after);
        } else {
            after.changed = true;
        }
        self.finish(after);
    }

    // ───────────────────────────── the line ─────────────────────────────

    pub fn show_alert(&self, alert: Rc<Alert>) {
        let mut after = After::default();
        {
            let mut s = self.s.borrow_mut();
            if !s.recording {
                s.samples.retain(|w| w.strong_count() > 0);
                s.samples.push(Rc::downgrade(&alert));
            }
            if s.suppressed && s.recording && !is_alarm(&alert) {
                // previews ask for the island to be shown first
                Self::remember(&mut s, &alert, true);
                after.calls.extend(alert.on_dismissed.clone());
                drop(s);
                self.finish(after);
                return;
            }
            if s.quiet && !alert.breaks_through_quiet {
                Self::remember(&mut s, &alert, true);
                // It won't be shown: e.g. an Island API script waiting for the result hears "dismissed".
                after.calls.extend(alert.on_dismissed.clone());
                drop(s);
                self.finish(after);
                return;
            }

            // Showing the alert that's already up (a HUD whose view was updated) just restarts its timer.
            if s.current.as_ref().is_some_and(|c| same(c, &alert)) {
                s.remaining = alert.duration;
                drop(s);
                self.changed();
                return;
            }
            if s.alerts.iter().any(|a| same(a, &alert)) {
                return;
            }

            if s.current.is_none() {
                drop(s);
                self.present(alert, &mut after);
            } else if alert.urgent {
                // Put the interrupted alert back at the front, unless it was a HUD itself (those are momentary).
                let waiting: Vec<Rc<Alert>> = s.alerts.drain(..).collect();
                let current = s.current.clone().unwrap();
                if !current.urgent {
                    s.alerts.push_back(current.clone());
                    let now = self.now();
                    s.queued_at.retain(|(a, _)| !same(a, &current));
                    s.queued_at.push((current, now)); // interrupted, not out of date
                }
                s.alerts.extend(waiting);
                drop(s);
                self.present(alert, &mut after);
            } else {
                let now = self.now();
                s.alerts.push_back(alert.clone());
                s.queued_at.push((alert, now));
            }
        }
        self.finish(after);
    }

    /// Closes the current alert (or, given one that's still waiting, takes it out of the line).
    pub fn dismiss_alert(&self, specific: Option<&Rc<Alert>>) {
        let mut after = After::default();
        {
            let mut s = self.s.borrow_mut();
            if let Some(specific) = specific
                && !s.current.as_ref().is_some_and(|c| same(c, specific)) {
                    // Remove it from the queue if it hasn't been shown yet.
                    s.alerts.retain(|a| !same(a, specific));
                    s.queued_at.retain(|(a, _)| !same(a, specific));
                    after.calls.extend(specific.on_dismissed.clone());
                    drop(s);
                    self.finish(after);
                    return;
                }
            let old = s.current.take();
            s.timer = None;
            if let Some(old) = old {
                after.calls.extend(old.on_dismissed.clone());
            }
        }
        self.present_next(&mut after);
        self.finish(after);
    }

    /// The next waiting alert. Ones that waited longer than [`MAX_WAIT`] are out of date: they go to Recent
    /// instead of popping up one after another (alarms excepted, they're still ringing).
    fn present_next(&self, after: &mut After) {
        loop {
            let next = {
                let mut s = self.s.borrow_mut();
                let Some(next) = s.alerts.pop_front() else { break };
                let queued = s.queued_at.iter().position(|(a, _)| same(a, &next)).map(|i| s.queued_at.remove(i).1).unwrap_or_else(|| self.now());
                if !is_alarm(&next) && self.now().saturating_duration_since(queued) > MAX_WAIT {
                    Self::remember(&mut s, &next, true);
                    after.calls.extend(next.on_dismissed.clone());
                    continue;
                }
                next
            };
            self.present(next, after);
            return;
        }
        after.changed = true;
    }

    fn present(&self, alert: Rc<Alert>, after: &mut After) {
        let mut s = self.s.borrow_mut();
        Self::remember(&mut s, &alert, false);
        s.remaining = alert.duration;
        s.current = Some(alert);
        s.last_tick = Instant::now();
        // Count real time: timer ticks can arrive late while the UI thread is busy, and counting ticks would then
        // stretch a 4 s alert to 10 s or more.
        let me = self.me.clone();
        s.timer = Some(Timer::every(Duration::from_millis(100), move || {
            if let Some(m) = me.upgrade() {
                m.tick();
            }
        }));
        after.changed = true;
    }

    fn tick(&self) {
        let expired = {
            let mut s = self.s.borrow_mut();
            let now = Instant::now();
            let elapsed = now.saturating_duration_since(s.last_tick);
            s.last_tick = now;
            let Some(a) = s.current.clone() else {
                s.timer = None;
                return;
            };
            if a.sticky.get() || a.sticky_until_hover.get() || s.hold {
                return;
            }
            s.remaining = s.remaining.saturating_sub(elapsed);
            s.remaining.is_zero()
        };
        if expired {
            self.dismiss_alert(None);
        }
    }

    /// Called by the window when the mouse touches the current alert.
    pub fn alert_hovered(&self) {
        let mut s = self.s.borrow_mut();
        if let Some(a) = s.current.clone()
            && a.sticky_until_hover.get() {
                a.sticky_until_hover.set(false);
                s.remaining = Duration::from_secs_f64(2.5);
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::el::El;
    use crate::ui::{NORMAL, text};

    fn alert(urgent: bool, breaks_through: bool) -> Rc<Alert> {
        let mut a = Alert::new(El::empty);
        a.urgent = urgent;
        a.breaks_through_quiet = breaks_through;
        Rc::new(a)
    }

    fn titled(title: &'static str) -> Alert {
        Alert::new(move || text(title, 14.0, NORMAL, crate::ui::colors::primary()))
    }

    fn is(a: &Option<Rc<Alert>>, b: &Rc<Alert>) -> bool {
        a.as_ref().is_some_and(|a| Rc::ptr_eq(a, b))
    }

    // ───────────── AlertQueueTests ─────────────

    #[test]
    fn shows_alerts_one_at_a_time_in_order() {
        let manager = Manager::new();
        let (first, second) = (alert(false, false), alert(false, false));
        manager.show_alert(first.clone());
        manager.show_alert(second.clone());

        assert!(is(&manager.current_alert(), &first));
        manager.dismiss_alert(None);
        assert!(is(&manager.current_alert(), &second));
        manager.dismiss_alert(None);
        assert!(manager.current_alert().is_none());
    }

    #[test]
    fn urgent_alert_jumps_ahead_and_the_interrupted_one_comes_back() {
        let manager = Manager::new();
        let (message, queued, volume) = (alert(false, false), alert(false, false), alert(true, false));
        manager.show_alert(message.clone());
        manager.show_alert(queued.clone());
        manager.show_alert(volume.clone());

        assert!(is(&manager.current_alert(), &volume));
        manager.dismiss_alert(None);
        assert!(is(&manager.current_alert(), &message)); // resumed before the rest of the line
        manager.dismiss_alert(None);
        assert!(is(&manager.current_alert(), &queued));
    }

    #[test]
    fn showing_the_current_alert_again_does_not_queue_it_twice() {
        let manager = Manager::new();
        let hud = alert(true, false);
        manager.show_alert(hud.clone());
        manager.show_alert(hud.clone());
        manager.show_alert(hud.clone());

        manager.dismiss_alert(None);
        assert!(manager.current_alert().is_none());
    }

    #[test]
    fn do_not_disturb_only_lets_through_alarms() {
        let manager = Manager::new();
        manager.set_quiet(true);
        manager.show_alert(alert(false, false));
        assert!(manager.current_alert().is_none());

        let alarm = alert(false, true);
        manager.show_alert(alarm.clone());
        assert!(is(&manager.current_alert(), &alarm));
    }

    #[test]
    fn dismissing_a_queued_alert_removes_it_from_the_line() {
        let manager = Manager::new();
        let (first, second, third) = (alert(false, false), alert(false, false), alert(false, false));
        manager.show_alert(first.clone());
        manager.show_alert(second.clone());
        manager.show_alert(third.clone());

        manager.dismiss_alert(Some(&second));
        manager.dismiss_alert(None);
        assert!(is(&manager.current_alert(), &third));
    }

    // ───────────── NewFeatureTests (island parts) ─────────────

    struct FakeActivity {
        kind: &'static str,
        priority: i32,
        active: Cell<bool>,
    }

    impl Activity for FakeActivity {
        fn name(&self) -> String {
            self.kind.into()
        }
        fn kind(&self) -> String {
            self.kind.into()
        }
        fn priority(&self) -> i32 {
            self.priority
        }
        fn is_active(&self) -> bool {
            self.active.get()
        }
        fn leading(&self) -> El {
            El::empty()
        }
        fn trailing(&self) -> El {
            El::empty()
        }
        fn bubble(&self) -> El {
            El::empty()
        }
        fn expanded(&self) -> El {
            El::empty()
        }
    }

    #[test]
    fn the_users_order_beats_the_built_in_priority() {
        let manager = Manager::new();
        let timer: Rc<dyn Activity> = Rc::new(FakeActivity { kind: "Timer", priority: 30, active: Cell::new(true) });
        let media: Rc<dyn Activity> = Rc::new(FakeActivity { kind: "Media", priority: 10, active: Cell::new(true) });
        manager.register(timer.clone());
        manager.register(media.clone());
        assert!(same_activity(&manager.active()[0], &timer)); // built-in

        manager.set_order(&["Media".to_string(), "Timer".to_string()]);
        assert!(same_activity(&manager.active()[0], &media));

        manager.set_order(&[]);
        assert!(same_activity(&manager.active()[0], &timer));
    }

    #[test]
    fn recent_keeps_shown_and_held_back_alerts_but_not_bars_or_previews() {
        let manager = Manager::new();
        manager.show_alert(titled("Shown").rc());
        manager.show_alert(titled("Volume bar").urgent().rc());
        manager.set_quiet(true);
        manager.show_alert(titled("Held back").rc());
        manager.set_quiet(false);
        manager.set_recording(false);
        manager.show_alert(titled("Preview").rc());
        manager.set_recording(true);

        let titles: Vec<String> = manager.history().iter().map(|h| h.title.clone()).collect();
        assert_eq!(titles, vec!["Held back", "Shown"]);
        assert!(manager.history()[0].missed);

        for i in 0..15 {
            let title: &'static str = Box::leak(format!("Alert {i}").into_boxed_str());
            manager.miss(titled(title).rc());
        }
        assert_eq!(manager.history().len(), 10);
    }

    #[test]
    fn showing_the_island_again_doesnt_replay_old_alerts() {
        let manager = Manager::new();
        // A sticky alert holds the line; normally the rest waits behind it.
        manager.show_alert(titled("Needs you").sticky_until_hover(true).rc());
        manager.show_alert(titled("Waiting 1").rc());
        manager.set_suppressed(true); // hidden
        manager.show_alert(titled("While hidden").rc());
        manager.set_suppressed(false); // shown again

        assert!(manager.current_alert().is_none()); // nothing old pops up
        let titles: Vec<String> = manager.history().iter().map(|h| h.title.clone()).collect();
        assert_eq!(titles, vec!["While hidden", "Waiting 1", "Needs you"]);
        assert!(manager.history().iter().take(2).all(|h| h.missed));
    }

    #[test]
    fn alerts_that_waited_too_long_go_to_recent() {
        let start = Instant::now();
        let now = Rc::new(Cell::new(start));
        let manager = Manager::new();
        let clock = now.clone();
        manager.set_clock(move || clock.get());

        let needs_you = titled("Needs you").sticky_until_hover(true).rc();
        manager.show_alert(needs_you.clone());
        manager.show_alert(titled("Old message").rc());
        now.set(start + Duration::from_secs(600)); // you were away
        let fresh = titled("Fresh message").rc();
        manager.show_alert(fresh.clone());
        manager.dismiss_alert(Some(&needs_you));

        assert!(is(&manager.current_alert(), &fresh));
        assert!(manager.history().iter().any(|h| h.title == "Old message" && h.missed));
    }

    #[test]
    fn a_ringing_timer_keeps_ringing_while_hidden() {
        let manager = Manager::new();
        let dismissed = Rc::new(Cell::new(false));
        let flag = dismissed.clone();
        let alarm = titled("Timer").sticky().breaks_through(true).on_dismissed(move || flag.set(true)).rc();
        manager.show_alert(alarm.clone());
        manager.set_suppressed(true);
        assert!(is(&manager.current_alert(), &alarm));
        assert!(!dismissed.get());
    }
}
