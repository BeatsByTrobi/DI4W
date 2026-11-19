//! "AirPods connected · 80%" when a paired Bluetooth device connects or disconnects, shown the way AirPods
//! connecting to an iPhone look: the device, "Connected", and its charge as a ring on the trailing side.

use crate::addons::{Addon, Base, Context};
use crate::model::Alert;
use crate::rt::Handle;
use crate::ui::colors;
use crate::ui::el::{Align, El, Kind};
use crate::ui::{self, ICON_SIZE, SEMIBOLD, icons};
use di4w_core::{AddonSettings, l};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use windows::Devices::Bluetooth::GenericAttributeProfile::{GattCharacteristicUuids, GattCommunicationStatus, GattServiceUuids};
use windows::Devices::Bluetooth::{BluetoothCacheMode, BluetoothLEDevice};
use windows::Devices::Enumeration::{DeviceInformation, DeviceInformationKind, DeviceWatcher, DeviceWatcherStatus};
use windows_collections::{IIterable, IMapView};
use windows::Foundation::{IPropertyValue, TypedEventHandler};
use windows::Storage::Streams::DataReader;
use windows::core::{HSTRING, IInspectable, Interface};

const IS_CONNECTED: &str = "System.Devices.Aep.IsConnected";
const IS_PAIRED: &str = "System.Devices.Aep.IsPaired";
const CATEGORY: &str = "System.Devices.Aep.Category";
const CONTAINER_ID: &str = "System.Devices.Aep.ContainerId";
/// Battery % for hands-free headsets.
const BATTERY_KEY: &str = "{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2";

// Classic Bluetooth and Bluetooth LE association endpoints.
const SELECTOR: &str = "System.Devices.Aep.ProtocolId:=\"{e0cbf06c-cd8b-4647-bb8a-263b43f0f974}\" OR System.Devices.Aep.ProtocolId:=\"{bb7bb05e-5972-42b5-94fc-76eaa7084d49}\"";

#[derive(Clone, Debug)]
pub struct Device {
    name: String,
    connected: bool,
    paired: bool,
    categories: Vec<String>,
    container: Option<String>,
    low_energy: bool,
}

enum Event {
    Added(String, Device),
    Updated(String, Option<bool>),
    Removed(String),
    Ready,
    Aborted,
}

fn value(props: &IMapView<HSTRING, IInspectable>, key: &str) -> Option<IPropertyValue> {
    props.Lookup(&HSTRING::from(key)).ok()?.cast().ok()
}

fn get_bool(props: &IMapView<HSTRING, IInspectable>, key: &str) -> Option<bool> {
    value(props, key)?.GetBoolean().ok()
}

fn device(info: &DeviceInformation) -> Option<(String, Device)> {
    let id = info.Id().ok()?.to_string();
    let props = info.Properties().ok()?;
    let categories = value(&props, CATEGORY)
        .and_then(|v| {
            let mut array = windows::core::Array::<HSTRING>::new();
            v.GetStringArray(&mut array).ok()?;
            Some(array.iter().map(|s| s.to_string()).collect())
        })
        .unwrap_or_default();
    let container = value(&props, CONTAINER_ID).and_then(|v| v.GetGuid().ok()).map(|g| format!("{g:?}"));
    Some((
        id.clone(),
        Device {
            name: info.Name().map(|n| n.to_string()).unwrap_or_default(),
            connected: get_bool(&props, IS_CONNECTED).unwrap_or(false),
            paired: get_bool(&props, IS_PAIRED).unwrap_or(false),
            categories,
            container,
            low_energy: id.to_lowercase().contains("bluetoothle"),
        },
    ))
}

/// The battery level of a Bluetooth device, when it shares one: headsets publish it as a device property,
/// Bluetooth LE devices through the GATT battery service. Blocks; call on a background thread.
pub fn read_battery(id: &str, container: Option<&str>, low_energy: bool, name: &str) -> Option<i32> {
    let result = (|| -> windows::core::Result<Option<i32>> {
        if let Some(c) = container {
            let props: IIterable<HSTRING> = vec![HSTRING::from(BATTERY_KEY)].into();
            let nodes = DeviceInformation::FindAllAsyncWithKindAqsFilterAndAdditionalProperties(&HSTRING::from(format!("System.Devices.ContainerId:=\"{{{}}}\"", c.trim_matches(['{', '}']))), &props, DeviceInformationKind::Device)?.join()?;
            for node in &nodes {
                if let Some(level) = node.Properties().ok().and_then(|p| value(&p, BATTERY_KEY)).and_then(|v| v.GetUInt8().ok()) {
                    return Ok(Some(level as i32));
                }
            }
        }
        if low_energy {
            let le = BluetoothLEDevice::FromIdAsync(&HSTRING::from(id))?.join()?;
            let services = le.GetGattServicesForUuidWithCacheModeAsync(GattServiceUuids::Battery()?, BluetoothCacheMode::Cached)?.join()?;
            for service in &services.Services()? {
                let chars = service.GetCharacteristicsForUuidWithCacheModeAsync(GattCharacteristicUuids::BatteryLevel()?, BluetoothCacheMode::Cached)?.join()?;
                for ch in &chars.Characteristics()? {
                    let read = ch.ReadValueWithCacheModeAsync(BluetoothCacheMode::Uncached)?.join()?;
                    if read.Status()? == GattCommunicationStatus::Success {
                        let buffer = read.Value()?;
                        if buffer.Length()? > 0 {
                            return Ok(Some(DataReader::FromBuffer(&buffer)?.ReadByte()? as i32));
                        }
                    }
                }
                let _ = service.Close();
            }
            let _ = le.Close();
        }
        Ok(None)
    })();
    result.unwrap_or_else(|e| {
        di4w_core::log::info(format!("No Bluetooth battery level for {name}: {}", e.message()));
        None
    })
}

pub struct Bluetooth {
    base: Base,
    devices: RefCell<HashMap<String, Device>>,
    ready: Cell<bool>,
    watcher: RefCell<Option<(DeviceWatcher, Handle<Event>)>>,
}

impl Bluetooth {
    pub fn new(ctx: &Rc<Context>) -> Rc<Bluetooth> {
        Rc::new(Bluetooth { base: Base::new(ctx, "Bluetooth", "Bluetooth"), devices: RefCell::new(HashMap::new()), ready: Cell::new(false), watcher: RefCell::new(None) })
    }

    fn on_event(self: &Rc<Self>, event: Event) {
        match event {
            Event::Added(id, device) => {
                self.devices.borrow_mut().insert(id.clone(), device.clone());
                if self.ready.get() && device.connected && device.paired {
                    self.announce(id, device, true);
                }
            }
            Event::Updated(id, connected) => {
                let Some(connected) = connected else { return };
                let device = {
                    let mut devices = self.devices.borrow_mut();
                    let Some(d) = devices.get_mut(&id) else { return };
                    if d.connected == connected {
                        return;
                    }
                    d.connected = connected;
                    d.clone()
                };
                if self.ready.get() && device.paired && !device.name.trim().is_empty() {
                    self.announce(id, device, connected);
                }
            }
            Event::Removed(id) => {
                self.devices.borrow_mut().remove(&id);
            }
            Event::Ready => self.ready.set(true), // don't announce what's already connected
            Event::Aborted => self.base.set_status(Some(l::t("Bluetooth is off or this PC has no Bluetooth.").into()), false),
        }
    }

    fn announce(self: &Rc<Self>, id: String, device: Device, connected: bool) {
        if !connected {
            self.show_connection(&device, false, None);
            return;
        }
        let me = Rc::downgrade(self);
        let d = device.clone();
        crate::rt::spawn(move || read_battery(&id, d.container.as_deref(), d.low_energy, &d.name), move |battery| {
            if let Some(b) = me.upgrade() {
                b.show_connection(&device, true, battery);
            }
        });
    }

    fn show_connection(&self, device: &Device, connected: bool, battery: Option<i32>) {
        let glyph = glyph_for(device);
        let name = device.name.clone();
        let alert = Alert::new(move || {
            let icon = ui::glyph_circle(glyph, if connected { colors::blue() } else { colors::button_bg() }, ICON_SIZE, 22.0);
            let subtitle = if connected { l::t("Connected") } else { l::t("Disconnected") };
            ui::alert_view(icon, &name, subtitle, None, battery.map(battery_ring))
        })
        .seconds(3.0)
        .keyline(if connected { Some(colors::blue()) } else { None });
        self.base.show(alert.rc());
    }
}

fn battery_ring(percent: i32) -> El {
    const SIZE: f64 = 44.0;
    const STROKE: f64 = 4.0;
    let color = if percent <= 20 { colors::red() } else { colors::green() };
    let label = ui::digits(percent.to_string(), 14.0, SEMIBOLD, colors::primary()).halign(Align::Center).tooltip(l::f("{0}% battery", &[&percent]));
    El::boxed(vec![El::new(Kind::Ellipse { stroke: colors::track(), thickness: STROKE }).size(SIZE, SIZE), icons::ring(percent as f64 / 100.0, SIZE, STROKE, color), label]).size(SIZE, SIZE)
}

fn glyph_for(d: &Device) -> &'static str {
    let all = format!("{} {}", d.categories.join(" "), d.name).to_lowercase();
    let has = |s: &str| all.contains(&s.to_lowercase());
    if ["Audio", "Headphone", "Headset", "Speaker", "Buds", "Pods"].iter().any(|s| has(s)) {
        "\u{E7F6}"
    } else if has("Mouse") {
        "\u{E962}"
    } else if has("Keyboard") {
        "\u{E765}"
    } else if has("Gaming") || has("Gamepad") || has("Controller") {
        "\u{E7FC}"
    } else if has("Phone") {
        "\u{E717}"
    } else {
        "\u{E702}"
    }
}

impl Addon for Bluetooth {
    fn base(&self) -> &Base {
        &self.base
    }

    fn enabled(&self, o: &AddonSettings) -> bool {
        o.bluetooth
    }

    fn start(self: Rc<Self>) -> Result<(), String> {
        self.devices.borrow_mut().clear();
        self.ready.set(false);
        let weak = Rc::downgrade(&self);
        let events = Handle::new(move |e| {
            if let Some(b) = weak.upgrade() {
                b.on_event(e);
            }
        });
        let props: IIterable<HSTRING> = vec![HSTRING::from(IS_CONNECTED), HSTRING::from(IS_PAIRED), HSTRING::from(CATEGORY), HSTRING::from(CONTAINER_ID)].into();
        let watcher = DeviceInformation::CreateWatcherWithKindAqsFilterAndAdditionalProperties(&HSTRING::from(SELECTOR), &props, DeviceInformationKind::AssociationEndpoint).map_err(|e| e.message())?;
        let (e1, e2, e3, e4, e5) = (events.clone(), events.clone(), events.clone(), events.clone(), events.clone());
        let r = (|| -> windows::core::Result<()> {
            watcher.Added(&TypedEventHandler::new(move |_, info: windows::core::Ref<DeviceInformation>| {
                if let Some((id, d)) = info.as_ref().and_then(device) {
                    e1.send(Event::Added(id, d));
                }
                Ok(())
            }))?;
            watcher.Updated(&TypedEventHandler::new(move |_, update: windows::core::Ref<windows::Devices::Enumeration::DeviceInformationUpdate>| {
                if let Some(u) = update.as_ref() {
                    let id = u.Id().map(|s| s.to_string()).unwrap_or_default();
                    let connected = u.Properties().ok().and_then(|p| get_bool(&p, IS_CONNECTED));
                    e2.send(Event::Updated(id, connected));
                }
                Ok(())
            }))?;
            watcher.Removed(&TypedEventHandler::new(move |_, update: windows::core::Ref<windows::Devices::Enumeration::DeviceInformationUpdate>| {
                if let Some(u) = update.as_ref() {
                    e3.send(Event::Removed(u.Id().map(|s| s.to_string()).unwrap_or_default()));
                }
                Ok(())
            }))?;
            watcher.EnumerationCompleted(&TypedEventHandler::new(move |_, _| {
                e4.send(Event::Ready);
                Ok(())
            }))?;
            watcher.Stopped(&TypedEventHandler::new(move |w: windows::core::Ref<DeviceWatcher>, _| {
                if w.as_ref().and_then(|w| w.Status().ok()) == Some(DeviceWatcherStatus::Aborted) {
                    e5.send(Event::Aborted);
                }
                Ok(())
            }))?;
            watcher.Start()
        })();
        r.map_err(|e| e.message())?;
        *self.watcher.borrow_mut() = Some((watcher, events));
        Ok(())
    }

    fn stop(&self) {
        if let Some((w, events)) = self.watcher.borrow_mut().take() {
            if matches!(w.Status(), Ok(DeviceWatcherStatus::Started) | Ok(DeviceWatcherStatus::EnumerationCompleted)) {
                let _ = w.Stop();
            }
            events.close();
        }
    }

    fn preview(self: Rc<Self>) {
        let sample = Device { name: l::t("Headphones").into(), connected: true, paired: true, categories: vec!["Audio".into()], container: None, low_energy: false };
        self.show_connection(&sample, true, Some(80));
    }
}
