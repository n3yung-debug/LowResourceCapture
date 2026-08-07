//! The two bits of native UI a windowed app can't do without.
//!
//! `clipanalyzer.exe` is built with `windows_subsystem = "windows"`, so it has
//! no console: `println!` and `eprintln!` go nowhere when it's launched from a
//! shortcut. Anything the user needs to see has to be an actual window, and
//! anything that goes wrong has to be shown rather than printed — otherwise the
//! app just vanishes, which is exactly how it failed on first release.

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Ask for a video file. `None` means the user cancelled.
pub fn pick_video() -> Option<String> {
    // Double-NUL terminated, NUL-separated pairs of (label, pattern).
    let filter: Vec<u16> =
        "Video files\0*.mp4;*.mkv;*.mov;*.avi;*.ts;*.webm\0All files\0*.*\0\0"
            .encode_utf16()
            .collect();
    let title = wide("Choose a recording to analyze");
    let mut buf = vec![0u16; 32 * 1024];

    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(buf.as_mut_ptr()),
        nMaxFile: buf.len() as u32,
        lpstrTitle: PCWSTR(title.as_ptr()),
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST,
        ..Default::default()
    };

    let picked = unsafe { GetOpenFileNameW(&mut ofn) };
    if !picked.as_bool() {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(0);
    if end == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..end]))
}

pub fn error(text: &str) {
    let t = wide(text);
    let c = wide("ClipAnalyzer");
    unsafe { MessageBoxW(None, PCWSTR(t.as_ptr()), PCWSTR(c.as_ptr()), MB_OK | MB_ICONERROR) };
}

pub fn info(text: &str) {
    let t = wide(text);
    let c = wide("ClipAnalyzer");
    unsafe {
        MessageBoxW(None, PCWSTR(t.as_ptr()), PCWSTR(c.as_ptr()), MB_OK | MB_ICONINFORMATION)
    };
}
