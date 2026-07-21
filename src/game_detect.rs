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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameEvent {
    Started { title: String, exe: String },
    Stopped,
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
