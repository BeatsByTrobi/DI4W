//! Screen reader support: the island window is a UI Automation text element whose name says what the island
//! shows ("Timer, 4:32", "Download complete, photos.zip"), and a polite live region, so new alerts (which otherwise
//! only appear visually) are read out. The C# app did the same with an invisible TextBlock and
//! `AutomationProperties.LiveSetting`.

use std::cell::RefCell;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::*;
use windows::core::{IUnknown, Result, implement};

thread_local! {
    static STATE: RefCell<(String, Option<IRawElementProviderSimple>)> = const { RefCell::new((String::new(), None)) };
}

#[implement(IRawElementProviderSimple)]
struct Provider {
    hwnd: HWND,
}

impl IRawElementProviderSimple_Impl for Provider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading)
    }

    fn GetPatternProvider(&self, _pattern: UIA_PATTERN_ID) -> Result<IUnknown> {
        Err(windows::core::Error::empty())
    }

    #[allow(non_upper_case_globals)]
    fn GetPropertyValue(&self, property: UIA_PROPERTY_ID) -> Result<VARIANT> {
        Ok(match property {
            UIA_NamePropertyId => {
                let text = STATE.try_with(|s| s.borrow().0.clone()).unwrap_or_default();
                VARIANT::from(text.as_str())
            }
            UIA_ControlTypePropertyId => VARIANT::from(UIA_TextControlTypeId.0),
            UIA_LiveSettingPropertyId => VARIANT::from(Polite.0),
            UIA_IsKeyboardFocusablePropertyId => VARIANT::from(false),
            _ => VARIANT::default(),
        })
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        unsafe { UiaHostProviderFromHwnd(self.hwnd) }
    }
}

fn provider(hwnd: HWND) -> IRawElementProviderSimple {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.1.get_or_insert_with(|| Provider { hwnd }.into()).clone()
    })
}

/// Answers `WM_GETOBJECT` for UI Automation.
pub fn get_object(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    if lparam.0 as i32 != UiaRootObjectId {
        return None;
    }
    let p = provider(hwnd);
    Some(unsafe { UiaReturnRawElementProvider(hwnd, wparam, lparam, &p) })
}

/// Sets what the island shows, in words; `announce` reads it out now (a new alert).
pub fn update(hwnd: HWND, text: &str, announce: bool) {
    let changed = STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.0 == text {
            return None;
        }
        let old = std::mem::replace(&mut s.0, text.to_string());
        Some(old)
    });
    // Only once a screen reader (or another UI Automation client) has asked for the island.
    let listening = unsafe { UiaClientsAreListening() }.as_bool();
    if !listening {
        return;
    }
    let p = provider(hwnd);
    unsafe {
        if let Some(old) = changed {
            let _ = UiaRaiseAutomationPropertyChangedEvent(&p, UIA_NamePropertyId, &VARIANT::from(old.as_str()), &VARIANT::from(text));
        }
        if announce {
            let _ = UiaRaiseAutomationEvent(&p, UIA_LiveRegionChangedEventId);
        }
    }
}
