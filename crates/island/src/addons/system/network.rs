//! "No internet connection", "Back online", and "Connected to HomeWiFi" when you switch networks.

use crate::addons::{Addon, Base, Context};
use crate::model::Alert;
use crate::rt::{Handle, Timer};
use crate::ui::colors::{self, Color};
use crate::ui::{self, ICON_SIZE};
use di4w_core::{AddonSettings, l};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use windows::Networking::Connectivity::{NetworkConnectivityLevel, NetworkInformation, NetworkStatusChangedEventHandler};

const WIFI: &str = "\u{E701}";
const ETHERNET: &str = "\u{E839}";

#[derive(Clone, Debug, PartialEq)]
struct State {
    online: bool,
    name: Option<String>,
    wireless: bool,
}

fn read() -> State {
    let r = (|| -> windows::core::Result<State> {
        let Ok(profile) = NetworkInformation::GetInternetConnectionProfile() else { return Ok(State { online: false, name: None, wireless: false }) };
        let online = profile.GetNetworkConnectivityLevel()? == NetworkConnectivityLevel::InternetAccess;
        let wireless = profile.IsWlanConnectionProfile().unwrap_or(false);
        let profile_name = profile.ProfileName().map(|s| s.to_string()).unwrap_or_default();
        let name = if wireless {
            profile.WlanConnectionProfileDetails().and_then(|d| d.GetConnectedSsid()).map(|s| s.to_string()).unwrap_or_default()
        } else {
            profile_name.clone()
        };
        let name = if name.trim().is_empty() { profile_name } else { name };
        Ok(State { online, name: if name.trim().is_empty() { None } else { Some(name) }, wireless })
    })();
    r.unwrap_or_else(|e| {
        di4w_core::log::info(format!("Network state unavailable: {}", e.message()));
        State { online: true, name: None, wireless: false }
    })
}

pub struct Network {
    base: Base,
    last: RefCell<Option<State>>,
    settle: RefCell<Option<Timer>>,
    token: RefCell<Option<(i64, Handle<()>)>>,
}

impl Network {
    pub fn new(ctx: &Rc<Context>) -> Rc<Network> {
        Rc::new(Network { base: Base::new(ctx, "Network", "Network"), last: RefCell::new(None), settle: RefCell::new(None), token: RefCell::new(None) })
    }

    fn compare(&self) {
        let now = read();
        let Some(before) = self.last.replace(Some(now.clone())) else { return };
        if now == before {
            return;
        }
        let glyph = if now.wireless { WIFI } else { ETHERNET };
        if !now.online && before.online {
            let subtitle = match &now.name {
                None => l::t("Check your Wi-Fi or cable").to_string(),
                Some(n) => l::f("Connected to {0}, but no internet", &[n]),
            };
            self.announce(l::t("No internet connection").into(), subtitle, glyph, colors::red());
        } else if now.online && !before.online {
            self.announce(l::t("Back online").into(), now.name.clone().unwrap_or_default(), glyph, colors::green());
        } else if now.online && now.name != before.name
            && let Some(n) = &now.name {
                self.announce(l::f("Connected to {0}", &[n]), if now.wireless { "Wi-Fi" } else { "Ethernet" }.into(), glyph, colors::blue());
            }
    }

    fn announce(&self, title: String, subtitle: String, glyph: &'static str, color: Color) {
        let alert = Alert::new(move || ui::alert_view(ui::glyph_circle(glyph, color, ICON_SIZE, 22.0), &title, &subtitle, None, None)).seconds(3.5);
        self.base.show(alert.rc());
    }
}

impl Addon for Network {
    fn base(&self) -> &Base {
        &self.base
    }

    fn enabled(&self, o: &AddonSettings) -> bool {
        o.network
    }

    fn start(self: Rc<Self>) -> Result<(), String> {
        *self.last.borrow_mut() = Some(read());
        let weak = Rc::downgrade(&self);
        // Network changes come in bursts (adapter down, up, DHCP, captive portal…); wait until it settles.
        let handle = Handle::new(move |()| {
            if let Some(n) = weak.upgrade() {
                let w = Rc::downgrade(&n);
                *n.settle.borrow_mut() = Some(Timer::once(Duration::from_secs(3), move || {
                    if let Some(n) = w.upgrade() {
                        n.compare();
                    }
                }));
            }
        });
        let h = handle.clone();
        let token = NetworkInformation::NetworkStatusChanged(&NetworkStatusChangedEventHandler::new(move |_| {
            h.send(());
            Ok(())
        }))
        .map_err(|e| e.message())?;
        *self.token.borrow_mut() = Some((token, handle));
        Ok(())
    }

    fn stop(&self) {
        if let Some((token, h)) = self.token.borrow_mut().take() {
            let _ = NetworkInformation::RemoveNetworkStatusChanged(token);
            h.close();
        }
        *self.settle.borrow_mut() = None;
    }

    fn preview(self: Rc<Self>) {
        self.announce(l::t("Back online").into(), "Wi-Fi".into(), WIFI, colors::green());
    }
}
