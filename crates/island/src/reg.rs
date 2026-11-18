//! Reading registry keys the add-ons need beyond `di4w_core::registry` (sub-key lists, QWORDs, HKLM).

use crate::rt::WideNull;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::*;
use windows::core::{PCWSTR, PWSTR};

/// An open registry key, closed when dropped.
pub struct Key(pub HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

// Raw handles can be moved to a watcher thread.
unsafe impl Send for Key {}

impl Key {
    pub fn open(root: HKEY, path: &str) -> Option<Key> {
        let path = path.encode_wide_null();
        let mut key = HKEY::default();
        let status = unsafe { RegOpenKeyExW(root, PCWSTR(path.as_ptr()), Some(0), KEY_READ | KEY_NOTIFY, &mut key) };
        (status == ERROR_SUCCESS).then_some(Key(key))
    }

    pub fn current_user(path: &str) -> Option<Key> {
        Key::open(HKEY_CURRENT_USER, path)
    }

    pub fn local_machine(path: &str) -> Option<Key> {
        Key::open(HKEY_LOCAL_MACHINE, path)
    }

    pub fn sub(&self, name: &str) -> Option<Key> {
        Key::open(self.0, name)
    }

    pub fn subkeys(&self) -> Vec<String> {
        let mut out = Vec::new();
        for i in 0.. {
            let mut buf = [0u16; 512];
            let mut len = buf.len() as u32;
            let status = unsafe { RegEnumKeyExW(self.0, i, Some(PWSTR(buf.as_mut_ptr())), &mut len, None, None, None, None) };
            if status != ERROR_SUCCESS {
                break;
            }
            out.push(String::from_utf16_lossy(&buf[..len as usize]));
        }
        out
    }

    /// A REG_QWORD value.
    pub fn qword(&self, name: &str) -> Option<u64> {
        let name = name.encode_wide_null();
        let mut kind = REG_VALUE_TYPE::default();
        let mut data = [0u8; 8];
        let mut size = 8u32;
        let status = unsafe { RegQueryValueExW(self.0, PCWSTR(name.as_ptr()), None, Some(&mut kind), Some(data.as_mut_ptr()), Some(&mut size)) };
        (status == ERROR_SUCCESS && kind == REG_QWORD && size == 8).then(|| u64::from_le_bytes(data))
    }
}
