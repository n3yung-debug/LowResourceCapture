//! Lightweight game auto-detection.
//!
//! Deliberately *not* a Medal-style online game database. Instead a cheap
//! foreground-window poll (a few times a second, negligible cost):
//!   * Get the foreground `HWND`, its process, and its executable name.
//!   * Treat it as a game if it's borderless/exclusive fullscreen on the
//!     primary monitor and using the GPU, OR its exe is on the user allowlist,
//!     UNLESS it's on the blocklist or is a known non-game (browsers, the
//!     shell, this app).
//!   * Emit `GameStarted { title }` / `GameStopped` transitions to the engine,
//!     which starts/stops the capture pipeline so nothing runs while you're on
//!     the desktop.
//!
//! Intended implementation (layer 5): `GetForegroundWindow` +
//! `GetWindowThreadProcessId` + `QueryFullProcessImageNameW`, compare window
//! rect to the monitor rect for fullscreen, debounce transitions.
//!
//! `UNVERIFIED` — needs Windows.

use crate::config::Config;

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HWND, MAX_PATH};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameEvent {
    Started { title: String, exe: String },
    Stopped,
}

/// Browser executables all collapse into a single "Browser" folder.
const BROWSERS: &[&str] = &[
    "chrome", "msedge", "firefox", "brave", "opera", "opera_gx", "vivaldi", "arc", "zen",
    "iexplore", "chromium",
];

/// Friendly subfolder name for the app currently in the foreground, used to
/// file clips under the output dir (e.g. `LowResourceCapture\Elden Ring\`,
/// `LowResourceCapture\Browser\`). Known browsers collapse to "Browser";
/// anything else uses its executable name. Never fails — falls back to
/// "Desktop" so a clip always lands somewhere sensible.
pub fn foreground_app_folder() -> String {
    match foreground_exe_stem() {
        Some(stem) => {
            if BROWSERS.contains(&stem.to_lowercase().as_str()) {
                "Browser".to_string()
            } else {
                folderize(&stem)
            }
        }
        None => "Desktop".to_string(),
    }
}

/// File stem (no `.exe`) of the foreground window's process image, e.g.
/// "eldenring" for `C:\...\eldenring.exe`.
fn foreground_exe_stem() -> Option<String> {
    unsafe {
        let hwnd: HWND = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false.into(), pid).ok()?;

        let mut buf = [0u16; MAX_PATH as usize];
        let mut size = buf.len() as u32;
        let res = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut size,
        );
        let _ = CloseHandle(handle);
        res.ok()?;

        let full = String::from_utf16_lossy(&buf[..size as usize]);
        std::path::Path::new(&full)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty())
    }
}

/// Turn an executable stem into a safe, readable folder name.
fn folderize(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        "Capture".to_string()
    } else {
        cleaned
    }
}

/// Decide, from an observed foreground window, whether it should count as a
/// game to capture. Pure logic split out so it *is* unit-testable without
/// Windows.
pub fn is_game(exe_lower: &str, is_fullscreen: bool, cfg: &Config) -> bool {
    if cfg.game_blocklist.iter().any(|b| b.eq_ignore_ascii_case(exe_lower)) {
        return false;
    }
    if NEVER_GAMES.contains(&exe_lower) {
        return false;
    }
    if cfg.game_allowlist.iter().any(|a| a.eq_ignore_ascii_case(exe_lower)) {
        return true;
    }
    // Default heuristic: exclusive/borderless fullscreen foreground app.
    is_fullscreen
}

/// Executables we never treat as games.
const NEVER_GAMES: &[&str] = &[
    "explorer.exe",
    "lowresourcecapture.exe",
    "chrome.exe",
    "firefox.exe",
    "msedge.exe",
    "devenv.exe",
    "code.exe",
    "discord.exe",
    "steam.exe",
    "steamwebhelper.exe",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fullscreen_unknown_app_is_game() {
        let cfg = Config::default();
        assert!(is_game("witcher3.exe", true, &cfg));
        assert!(!is_game("witcher3.exe", false, &cfg));
    }

    #[test]
    fn blocklist_wins_over_fullscreen() {
        let mut cfg = Config::default();
        cfg.game_blocklist.push("mpv.exe".into());
        assert!(!is_game("mpv.exe", true, &cfg));
    }

    #[test]
    fn allowlist_forces_windowed_game() {
        let mut cfg = Config::default();
        cfg.game_allowlist.push("osu!.exe".into());
        assert!(is_game("osu!.exe", false, &cfg));
    }

    #[test]
    fn browsers_never_games() {
        let cfg = Config::default();
        assert!(!is_game("chrome.exe", true, &cfg));
    }
}
