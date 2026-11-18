//! Small reads and writes under HKEY_CURRENT_USER.

use crate::wide::{pcwstr, wide};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ, REG_VALUE_TYPE,
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn open(path: &str) -> Option<Key> {
    let path = wide(path);
    let mut key = HKEY::default();
    let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, pcwstr(&path), Some(0), KEY_READ, &mut key) };
    (status == ERROR_SUCCESS).then_some(Key(key))
}

/// The raw value and its type, or `None` when the key or value is missing.
fn query(path: &str, name: &str) -> Option<(REG_VALUE_TYPE, Vec<u8>)> {
    let key = open(path)?;
    let name = wide(name);
    let mut kind = REG_VALUE_TYPE::default();
    let mut size = 0u32;
    unsafe {
        if RegQueryValueExW(key.0, pcwstr(&name), None, Some(&mut kind), None, Some(&mut size)) != ERROR_SUCCESS {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        if RegQueryValueExW(key.0, pcwstr(&name), None, Some(&mut kind), Some(data.as_mut_ptr()), Some(&mut size))
            != ERROR_SUCCESS
        {
            return None;
        }
        data.truncate(size as usize);
        Some((kind, data))
    }
}

pub fn read_dword(path: &str, name: &str) -> Option<u32> {
    match query(path, name)? {
        (kind, data) if kind == REG_DWORD && data.len() >= 4 => Some(u32::from_le_bytes([data[0], data[1], data[2], data[3]])),
        _ => None,
    }
}

pub fn read_string(path: &str, name: &str) -> Option<String> {
    let (kind, data) = query(path, name)?;
    if kind != REG_SZ {
        return None;
    }
    let units: Vec<u16> = data.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some(crate::wide::from_wide(&units))
}

pub fn write_string(path: &str, name: &str, value: &str) -> windows::core::Result<()> {
    let path = wide(path);
    let mut key = HKEY::default();
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            pcwstr(&path),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
        .ok()?;
        let key = Key(key);
        let name = wide(name);
        let value = wide(value);
        let bytes = std::slice::from_raw_parts(value.as_ptr().cast::<u8>(), value.len() * 2);
        RegSetValueExW(key.0, pcwstr(&name), None, REG_SZ, Some(bytes)).ok()
    }
}

/// Removes a value; a value that's already gone counts as done.
pub fn delete_value(path: &str, name: &str) {
    let path = wide(path);
    let mut key = HKEY::default();
    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, pcwstr(&path), Some(0), KEY_SET_VALUE, &mut key) == ERROR_SUCCESS {
            let key = Key(key);
            let name = wide(name);
            let _ = RegDeleteValueW(key.0, pcwstr(&name));
        }
    }
}
