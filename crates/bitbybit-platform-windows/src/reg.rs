//! Read-only HKLM registry helpers (64-bit view; this crate is x64 only). Missing or unreadable values are `None`.

use windows::Win32::Foundation::{ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_READ, REG_DWORD, REG_ROUTINE_FLAGS, REG_SZ, REG_VALUE_TYPE, RRF_RT_REG_BINARY,
    RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegCloseKey, RegDeleteKeyValueW, RegEnumKeyExW, RegEnumValueW, RegGetValueW,
    RegOpenKeyExW, RegSetKeyValueW,
};
use windows::core::{HSTRING, PWSTR};

fn bytes(path: &str, value: &str, flags: REG_ROUTINE_FLAGS) -> Option<Vec<u8>> {
    let (key, name) = (HSTRING::from(path), HSTRING::from(value));
    let mut len = 0u32;
    // SAFETY: size query, then a buffer of that size; the type is restricted by `flags`.
    unsafe {
        if RegGetValueW(HKEY_LOCAL_MACHINE, &key, &name, flags, None, None, Some(&mut len)) != ERROR_SUCCESS {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        if RegGetValueW(HKEY_LOCAL_MACHINE, &key, &name, flags, None, Some(buf.as_mut_ptr().cast()), Some(&mut len))
            != ERROR_SUCCESS
        {
            return None;
        }
        buf.truncate(len as usize);
        Some(buf)
    }
}

fn utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let end = units.iter().position(|c| *c == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

/// REG_SZ (or expandable, unexpanded); blank values are `None`.
pub fn string(path: &str, value: &str) -> Option<String> {
    bytes(path, value, RRF_RT_REG_SZ).map(|b| utf16(&b)).filter(|s| !s.trim().is_empty())
}

pub fn dword(path: &str, value: &str) -> Option<u32> {
    bytes(path, value, RRF_RT_REG_DWORD).and_then(|b| Some(u32::from_le_bytes(b.get(..4)?.try_into().ok()?)))
}

pub fn binary(path: &str, value: &str) -> Option<Vec<u8>> {
    bytes(path, value, RRF_RT_REG_BINARY).filter(|b| !b.is_empty())
}

struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: opened by `open`.
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

fn open(path: &str) -> Option<Key> {
    let mut key = HKEY::default();
    // SAFETY: read-only open; closed by `Key`.
    (unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, &HSTRING::from(path), None, KEY_READ, &mut key) } == ERROR_SUCCESS)
        .then_some(Key(key))
}

pub fn exists(path: &str) -> bool {
    open(path).is_some()
}

pub fn subkeys(path: &str) -> Vec<String> {
    let Some(key) = open(path) else { return vec![] };
    let mut out = Vec::new();
    for index in 0..4096u32 {
        let mut name = [0u16; 256];
        let mut len = name.len() as u32;
        // SAFETY: buffer of `len` UTF-16 units.
        let code: WIN32_ERROR =
            unsafe { RegEnumKeyExW(key.0, index, Some(PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None) };
        if code != ERROR_SUCCESS {
            break; // ERROR_NO_MORE_ITEMS (or an unreadable entry): stop, never loop forever
        }
        out.push(String::from_utf16_lossy(&name[..len as usize]));
    }
    out
}

/// (name, text) of the REG_SZ values of a key; other types are skipped.
pub fn string_values(path: &str) -> Vec<(String, String)> {
    let Some(key) = open(path) else { return vec![] };
    let mut out = Vec::new();
    for index in 0..4096u32 {
        let mut name = [0u16; 256];
        let mut name_len = name.len() as u32;
        let mut data = vec![0u8; 2048];
        let mut data_len = data.len() as u32;
        let mut kind = REG_VALUE_TYPE::default();
        // SAFETY: buffers of the given lengths.
        let code = unsafe {
            RegEnumValueW(
                key.0,
                index,
                Some(PWSTR(name.as_mut_ptr())),
                &mut name_len,
                None,
                Some(&mut kind.0),
                Some(data.as_mut_ptr()),
                Some(&mut data_len),
            )
        };
        if code != ERROR_SUCCESS {
            break; // no more values, or one larger than any hardware id: stop
        }
        if kind == REG_SZ {
            out.push((String::from_utf16_lossy(&name[..name_len as usize]), utf16(&data[..data_len as usize])));
        }
    }
    out
}

fn written(api: &str, code: WIN32_ERROR) -> bitbybit_core::Result<()> {
    if code == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(bitbybit_core::Error::platform(format!("{api} failed"), code.0 as i32))
    }
}

/// Writes a REG_DWORD under HKLM (the key is created when missing). Callers pass fixed, owned paths only.
pub fn set_dword(path: &str, value: &str, data: u32) -> bitbybit_core::Result<()> {
    let bytes = data.to_le_bytes();
    // SAFETY: 4 bytes of REG_DWORD data.
    written("RegSetKeyValueW", unsafe {
        RegSetKeyValueW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(path),
            &HSTRING::from(value),
            REG_DWORD.0,
            Some(bytes.as_ptr().cast()),
            4,
        )
    })
}

/// Writes a REG_SZ under HKLM (the key is created when missing).
pub fn set_string(path: &str, value: &str, data: &str) -> bitbybit_core::Result<()> {
    let wide: Vec<u16> = data.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: NUL-terminated UTF-16 data of the given byte length.
    written("RegSetKeyValueW", unsafe {
        RegSetKeyValueW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(path),
            &HSTRING::from(value),
            REG_SZ.0,
            Some(wide.as_ptr().cast()),
            (wide.len() * 2) as u32,
        )
    })
}

/// Deletes one value under HKLM; a value that is already gone is not an error.
pub fn delete_value(path: &str, value: &str) -> bitbybit_core::Result<()> {
    // SAFETY: fixed key path and value name.
    let code = unsafe { RegDeleteKeyValueW(HKEY_LOCAL_MACHINE, &HSTRING::from(path), &HSTRING::from(value)) };
    if code == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND { Ok(()) } else { written("RegDeleteKeyValueW", code) }
}
