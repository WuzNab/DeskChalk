//! HKEY_CURRENT_USER registry helpers. DeskChalk never touches machine-wide keys.

use super::util::{from_wide, wide};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegGetValueW, RegOpenKeyExW,
    RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, KEY_WRITE,
    REG_OPTION_NON_VOLATILE, REG_ROUTINE_FLAGS, REG_SZ, REG_VALUE_TYPE, RRF_RT_REG_BINARY,
    RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};

pub fn set_sz(path: &str, name: &str, value: &str) -> bool {
    let data: Vec<u8> = wide(value).iter().flat_map(|c| c.to_le_bytes()).collect();
    set_raw(path, name, REG_SZ, &data)
}

fn set_raw(path: &str, name: &str, ty: REG_VALUE_TYPE, data: &[u8]) -> bool {
    unsafe {
        let mut key = HKEY::default();
        let created = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(path),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        );
        if created != ERROR_SUCCESS {
            return false;
        }
        let ok = RegSetValueExW(key, &HSTRING::from(name), None, ty, Some(data)) == ERROR_SUCCESS;
        let _ = RegCloseKey(key);
        ok
    }
}

fn get_raw(path: &str, name: &str, flags: REG_ROUTINE_FLAGS) -> Option<Vec<u8>> {
    unsafe {
        let (p, n) = (HSTRING::from(path), HSTRING::from(name));
        let mut size = 0u32;
        if RegGetValueW(HKEY_CURRENT_USER, &p, &n, flags, None, None, Some(&mut size)) != ERROR_SUCCESS {
            return None;
        }
        let mut buf = vec![0u8; size as usize + 2];
        size = buf.len() as u32;
        let r = RegGetValueW(
            HKEY_CURRENT_USER,
            &p,
            &n,
            flags,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        );
        if r != ERROR_SUCCESS {
            return None;
        }
        buf.truncate(size as usize);
        Some(buf)
    }
}

pub fn get_sz(path: &str, name: &str) -> Option<String> {
    let bytes = get_raw(path, name, RRF_RT_REG_SZ)?;
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    Some(from_wide(&units))
}

pub fn get_dword(path: &str, name: &str) -> Option<u32> {
    let bytes = get_raw(path, name, RRF_RT_REG_DWORD)?;
    Some(u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?))
}

pub fn get_binary(path: &str, name: &str) -> Option<Vec<u8>> {
    get_raw(path, name, RRF_RT_REG_BINARY)
}

pub fn delete_value(path: &str, name: &str) {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(path), None, KEY_SET_VALUE, &mut key)
            == ERROR_SUCCESS
        {
            let _ = RegDeleteValueW(key, &HSTRING::from(name));
            let _ = RegCloseKey(key);
        }
    }
}

