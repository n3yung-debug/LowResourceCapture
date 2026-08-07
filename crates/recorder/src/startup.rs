//! Run-at-Windows-startup toggle, backed by the per-user Run registry key.
//! Toggled from the settings GUI (so it can change without reinstalling).

use anyhow::{Context, Result};

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "LowResourceCapture";

/// Is the app currently set to launch at login?
pub fn is_enabled() -> bool {
    let subkey = HSTRING::from(RUN_KEY);
    let value = HSTRING::from(VALUE_NAME);
    let mut cb: u32 = 0;
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut cb as *mut u32),
        )
    };
    rc == ERROR_SUCCESS
}

/// Enable or disable launch at login for the current user.
pub fn set_enabled(enabled: bool) -> Result<()> {
    let subkey = HSTRING::from(RUN_KEY);
    let value = HSTRING::from(VALUE_NAME);
    if enabled {
        let exe = std::env::current_exe().context("locating current exe")?;
        let cmd = format!("\"{}\"", exe.to_string_lossy());
        let wide: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
        let rc = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                PCWSTR(value.as_ptr()),
                REG_SZ.0,
                Some(wide.as_ptr() as *const _),
                (wide.len() * 2) as u32,
            )
        };
        if rc != ERROR_SUCCESS {
            anyhow::bail!("RegSetKeyValueW failed: {}", rc.0);
        }
    } else {
        let rc = unsafe {
            RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                PCWSTR(value.as_ptr()),
            )
        };
        if rc != ERROR_SUCCESS && rc != ERROR_FILE_NOT_FOUND {
            anyhow::bail!("RegDeleteKeyValueW failed: {}", rc.0);
        }
    }
    Ok(())
}
