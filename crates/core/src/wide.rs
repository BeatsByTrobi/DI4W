//! UTF-16 helpers for Win32 calls.

use windows::core::PCWSTR;

/// A NUL-terminated UTF-16 copy of `text`; keep it alive while the `PCWSTR` from [`pcwstr`] is used.
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn pcwstr(buffer: &[u16]) -> PCWSTR {
    PCWSTR(buffer.as_ptr())
}

/// Text from a fixed-size UTF-16 buffer, up to its first NUL.
pub fn from_wide(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}
