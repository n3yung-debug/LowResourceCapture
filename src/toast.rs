//! "Clip saved" desktop toast notifications.
//!
//! Uses the WinRT toast API. For an unpackaged app to be allowed to toast, the
//! process needs an AppUserModelID that's registered — we set the process AUMID
//! and register it in HKCU (`init`), and the installer also stamps the same
//! AUMID on the Start Menu shortcut. All best-effort: a failure just logs.

use anyhow::Result;

use windows::core::{HSTRING, PCWSTR};
use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager, ToastTemplateType};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;

/// Stable app id used for toasts and the Start Menu shortcut.
pub const AUMID: &str = "LowResourceCapture.App";

/// Set + register the AUMID so this (unpackaged) app may show toasts.
pub fn init() {
    let aumid = HSTRING::from(AUMID);
    unsafe {
        let _ = SetCurrentProcessExplicitAppUserModelID(PCWSTR(aumid.as_ptr()));
    }
    // HKCU\Software\Classes\AppUserModelId\<AUMID> : DisplayName
    let subkey = HSTRING::from(format!(r"Software\Classes\AppUserModelId\{AUMID}"));
    let name = HSTRING::from("DisplayName");
    let disp: Vec<u16> = "LowResourceCapture"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let rc = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(name.as_ptr()),
            REG_SZ.0,
            Some(disp.as_ptr() as *const _),
            (disp.len() * 2) as u32,
        )
    };
    if rc != ERROR_SUCCESS {
        log::warn!("could not register toast AUMID: {}", rc.0);
    }
}

/// Show a two-line toast. Best-effort.
pub fn show(title: &str, body: &str) {
    if let Err(e) = try_show(title, body) {
        log::warn!("toast failed: {e:#}");
    }
}

fn try_show(title: &str, body: &str) -> Result<()> {
    let xml: XmlDocument =
        ToastNotificationManager::GetTemplateContent(ToastTemplateType::ToastText02)?;
    let texts = xml.GetElementsByTagName(&HSTRING::from("text"))?;
    texts.Item(0)?.SetInnerText(&HSTRING::from(title))?;
    texts.Item(1)?.SetInnerText(&HSTRING::from(body))?;

    let toast = ToastNotification::CreateToastNotification(&xml)?;
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?;
    notifier.Show(&toast)?;
    Ok(())
}
