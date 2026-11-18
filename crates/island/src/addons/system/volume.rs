//! Volume bar when the output volume or mute changes, from any source (keys, mixer, apps). Follows the default
//! output when it changes (speakers → headphones).

use crate::addons::hud::Hud;
use crate::addons::{Addon, Base, Context};
use crate::rt::Handle;
use crate::ui::colors;
use di4w_core::{AddonSettings, l};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::core::{PCWSTR, implement};

const SPEAKER: &str = "\u{E767}";
const MUTED: &str = "\u{E74F}";

enum Event {
    Volume(f32, bool),
    DefaultChanged,
}

#[implement(IAudioEndpointVolumeCallback)]
struct VolumeCallback(Handle<Event>);

impl IAudioEndpointVolumeCallback_Impl for VolumeCallback_Impl {
    // Called on a Core Audio thread.
    fn OnNotify(&self, data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows::core::Result<()> {
        if let Some(d) = unsafe { data.as_ref() } {
            self.0.send(Event::Volume(d.fMasterVolume, d.bMuted.as_bool()));
        }
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DeviceCallback(Handle<Event>);

impl IMMNotificationClient_Impl for DeviceCallback_Impl {
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnDeviceAdded(&self, _: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnDeviceRemoved(&self, _: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }
    // The default output (speakers -> headphones) changed: follow it.
    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _: &PCWSTR) -> windows::core::Result<()> {
        if flow == eRender && role == eMultimedia {
            self.0.send(Event::DefaultChanged);
        }
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> windows::core::Result<()> {
        Ok(())
    }
}

struct State {
    enumerator: Option<IMMDeviceEnumerator>,
    device_callback: Option<IMMNotificationClient>,
    endpoint: Option<(IAudioEndpointVolume, IAudioEndpointVolumeCallback)>,
    events: Option<Handle<Event>>,
    quiet_until: Instant,
}

pub struct Volume {
    base: Base,
    hud: Hud,
    st: RefCell<State>,
}

impl Volume {
    pub fn new(ctx: &Rc<Context>) -> Rc<Volume> {
        Rc::new(Volume {
            base: Base::new(ctx, "Volume", "Volume"),
            hud: Hud::new(SPEAKER, colors::primary()),
            st: RefCell::new(State { enumerator: None, device_callback: None, endpoint: None, events: None, quiet_until: Instant::now() }),
        })
    }

    fn attach(&self) {
        self.detach();
        let mut st = self.st.borrow_mut();
        let (Some(enumerator), Some(events)) = (st.enumerator.clone(), st.events.clone()) else { return };
        let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia) };
        let Ok(device) = device else {
            drop(st);
            self.base.set_status(Some(l::t("No speakers or headphones found.").into()), false);
            return;
        };
        drop(st);
        self.base.set_status(None, false);
        let attached = unsafe {
            device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).and_then(|volume| {
                let callback: IAudioEndpointVolumeCallback = VolumeCallback(events).into();
                volume.RegisterControlChangeNotify(&callback)?;
                Ok((volume, callback))
            })
        };
        st = self.st.borrow_mut();
        st.endpoint = attached.ok();
        st.quiet_until = Instant::now() + Duration::from_secs(1); // switching devices fires a notification; don't show it
    }

    fn detach(&self) {
        if let Some((volume, callback)) = self.st.borrow_mut().endpoint.take() {
            unsafe {
                let _ = volume.UnregisterControlChangeNotify(&callback);
            }
        }
    }

    fn on_event(&self, event: Event) {
        match event {
            Event::DefaultChanged => {
                if self.st.borrow().enumerator.is_some() {
                    self.attach();
                }
            }
            Event::Volume(level, muted) => {
                if Instant::now() < self.st.borrow().quiet_until || self.st.borrow().enumerator.is_none() {
                    return;
                }
                let muted = muted || level <= 0.001;
                let percent = (level as f64 * 100.0).round_ties_even();
                if muted {
                    self.hud.set(None, l::t("Muted"), Some(MUTED));
                } else {
                    self.hud.set(Some(level as f64), &format!("{percent}%"), Some(SPEAKER));
                }
                self.base.show(self.hud.alert.clone());
            }
        }
    }
}

impl Addon for Volume {
    fn base(&self) -> &Base {
        &self.base
    }

    fn enabled(&self, o: &AddonSettings) -> bool {
        o.volume
    }

    fn start(self: Rc<Self>) -> Result<(), String> {
        let weak = Rc::downgrade(&self);
        let events = Handle::new(move |e| {
            if let Some(v) = weak.upgrade() {
                v.on_event(e);
            }
        });
        unsafe {
            let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.message())?;
            let client: IMMNotificationClient = DeviceCallback(events.clone()).into();
            enumerator.RegisterEndpointNotificationCallback(&client).map_err(|e| e.message())?;
            let mut st = self.st.borrow_mut();
            st.enumerator = Some(enumerator);
            st.device_callback = Some(client);
            st.events = Some(events);
        }
        self.attach();
        Ok(())
    }

    fn stop(&self) {
        self.detach();
        let mut st = self.st.borrow_mut();
        if let (Some(e), Some(c)) = (st.enumerator.take(), st.device_callback.take()) {
            unsafe {
                let _ = e.UnregisterEndpointNotificationCallback(&c);
            }
        }
        if let Some(h) = st.events.take() {
            h.close();
        }
    }

    fn preview(self: Rc<Self>) {
        self.hud.set(Some(0.6), "60", Some(SPEAKER));
        self.base.show(self.hud.alert.clone());
    }
}
