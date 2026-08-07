//! Global hotkey registration.
//!
//! Each clip preset registers a system-wide hotkey. When one fires we look up
//! how many seconds that preset saves and hand it to the capture engine.
//!
//! Uses the `global-hotkey` crate (a thin Win32 `RegisterHotKey` wrapper —
//! no runtime cost while idle; the OS delivers a message when the combo is
//! pressed). Events are drained from `GlobalHotKeyEvent::receiver()` on the
//! main message loop.

use anyhow::{anyhow, Result};
use global_hotkey::{hotkey::HotKey, GlobalHotKeyManager};
use std::collections::HashMap;
use std::str::FromStr;

use crate::config::Config;

pub struct HotkeyRouter {
    _manager: GlobalHotKeyManager,
    /// Map from the OS hotkey id to the save length in seconds.
    id_to_seconds: HashMap<u32, u32>,
    /// Map from the OS hotkey id to the preset label (for logging/toasts).
    id_to_name: HashMap<u32, String>,
}

impl HotkeyRouter {
    /// Register every preset's hotkey. Bad/duplicate combos are logged and
    /// skipped rather than aborting startup, so one typo doesn't kill the app.
    pub fn register(config: &Config) -> Result<Self> {
        let manager = GlobalHotKeyManager::new()
            .map_err(|e| anyhow!("creating hotkey manager: {e}"))?;
        let mut id_to_seconds = HashMap::new();
        let mut id_to_name = HashMap::new();

        for preset in &config.presets {
            let hotkey = match HotKey::from_str(&preset.hotkey) {
                Ok(h) => h,
                Err(e) => {
                    log::warn!(
                        "skipping preset '{}': unparseable hotkey '{}': {e}",
                        preset.name,
                        preset.hotkey
                    );
                    continue;
                }
            };
            if let Err(e) = manager.register(hotkey) {
                log::warn!(
                    "skipping preset '{}': could not register '{}' (already in use?): {e}",
                    preset.name,
                    preset.hotkey
                );
                continue;
            }
            id_to_seconds.insert(hotkey.id(), preset.seconds);
            id_to_name.insert(hotkey.id(), preset.name.clone());
            log::info!(
                "registered '{}' -> save last {}s ({})",
                preset.hotkey,
                preset.seconds,
                preset.name
            );
        }

        if id_to_seconds.is_empty() {
            return Err(anyhow!(
                "no hotkeys registered; check the [presets] section of config.toml"
            ));
        }

        Ok(Self {
            _manager: manager,
            id_to_seconds,
            id_to_name,
        })
    }

    /// Resolve a fired hotkey id to (seconds, preset name), if we own it.
    pub fn resolve(&self, hotkey_id: u32) -> Option<(u32, &str)> {
        let secs = *self.id_to_seconds.get(&hotkey_id)?;
        let name = self.id_to_name.get(&hotkey_id).map(String::as_str).unwrap_or("");
        Some((secs, name))
    }
}
