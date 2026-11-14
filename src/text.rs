//! UTF-16 helpers for Win32 string parameters.

/// Encode a string as a null-terminated UTF-16 buffer.
pub fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}
