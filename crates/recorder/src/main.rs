// LowResourceCapture — retroactive game clip recorder for Windows.
//
// Release builds hide the console window; debug builds keep it for logs.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod audio;
mod capture;
mod convert;
mod encoder;
mod engine;
mod game_detect;
mod gui;
mod hotkeys;
mod muxer;
mod ringbuffer;
mod startup;
mod stats;
mod toast;

// Config, logging, the ffmpeg wrappers and the clip editor now live in the
// `shared` crate so the analyzer can use them too. Re-exporting them at the
// crate root keeps every existing `crate::config::…` path resolving unchanged.
pub use shared::{clips, config, ffmpeg, logging};

/// Custom thread message: settings GUI closed → reload config + hotkeys.
const WM_RELOAD: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 1;

use anyhow::Result;
use global_hotkey::{GlobalHotKeyEvent, HotKeyState};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    TrayIconBuilder,
};

use crate::config::Config;
use crate::engine::EngineCommand;
use crate::hotkeys::HotkeyRouter;

fn main() {
    if let Err(e) = run() {
        // Surface fatal errors even in the windowed (no-console) build.
        log::error!("fatal: {e:#}");
        #[cfg(windows)]
        show_error_box(&format!("LowResourceCapture failed to start:\n\n{e:#}"));
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    // Second-process modes: show a window (settings or clip library) instead of
    // the tray app.
    let is_gui = std::env::args().any(|a| a == "--gui");
    let is_clips = std::env::args().any(|a| a == "--clips");
    // Main process starts a fresh log; window subprocesses append so they don't
    // wipe the main log.
    init_logging(!(is_gui || is_clips));

    if is_gui {
        log::info!("launching settings GUI");
        return gui::run();
    }
    if is_clips {
        log::info!("launching clip library");
        return clips::run();
    }

    log::info!("LowResourceCapture starting");

    // Register the app id so "clip saved" toasts are allowed (best-effort).
    toast::init();

    let config = Config::load_or_create()?;
    std::fs::create_dir_all(&config.output_dir).ok();

    // Start the capture engine (idle until a game is detected).
    let (engine_handle, engine_tx) = engine::spawn(config.clone());

    // Register the clip hotkeys. Held in an Option so we can drop-and-rebuild
    // them live when settings change.
    let mut router = Some(HotkeyRouter::register(&config)?);

    // Build the tray icon + menu.
    let tray = build_tray(&config)?;

    // Capture continuously while the app is open, so the last N seconds are
    // always ready to clip — no manual start needed. (Dropping/quitting stops
    // it; the debug menu can still start/stop manually.)
    let _ = engine_tx.send(EngineCommand::StartCapture {
        window_title: "(auto: primary monitor)".to_string(),
        with_audio: true,
    });

    log::info!("ready; capturing. Press a clip hotkey to save the last N seconds.");

    // Pump the Win32 message loop and route hotkey + menu events.
    run_message_loop(&mut router, &engine_tx, &tray)?;

    engine_handle.shutdown();
    log::info!("exited cleanly");
    Ok(())
}

/// The tray menu items we need to compare events against.
struct Tray {
    _icon: tray_icon::TrayIcon,
    settings_id: tray_icon::menu::MenuId,
    clips_id: tray_icon::menu::MenuId,
    open_clips_id: tray_icon::menu::MenuId,
    quit_id: tray_icon::menu::MenuId,
    output_dir: std::path::PathBuf,
}

fn build_tray(config: &Config) -> Result<Tray> {
    let menu = Menu::new();
    let clips = MenuItem::new("Clip library…", true, None);
    let settings = MenuItem::new("Settings…", true, None);
    let open_clips = MenuItem::new("Open clips folder", true, None);
    let quit = MenuItem::new("Quit", true, None);

    menu.append(&clips)?;
    menu.append(&settings)?;
    menu.append(&open_clips)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&quit)?;

    let icon = make_tray_icon();

    let tray_icon = TrayIconBuilder::new()
        .with_tooltip("LowResourceCapture — press a clip hotkey in-game")
        .with_menu(Box::new(menu))
        .with_icon(icon)
        .build()?;

    Ok(Tray {
        _icon: tray_icon,
        settings_id: settings.id().clone(),
        clips_id: clips.id().clone(),
        open_clips_id: open_clips.id().clone(),
        quit_id: quit.id().clone(),
        output_dir: config.output_dir.clone(),
    })
}

/// The app icon (32x32 RGBA), generated from `assets/icon.ico` and embedded so
/// the tray icon matches the exe/installer icon with no runtime asset files.
fn make_tray_icon() -> tray_icon::Icon {
    // Repo-root `assets/`, three levels up from crates/recorder/src/.
    const RGBA: &[u8] = include_bytes!("../../../assets/tray-32.rgba");
    tray_icon::Icon::from_rgba(RGBA.to_vec(), 32, 32).expect("building tray icon")
}

fn run_message_loop(
    router: &mut Option<HotkeyRouter>,
    engine_tx: &std::sync::mpsc::Sender<EngineCommand>,
    tray: &Tray,
) -> Result<()> {
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, SetTimer, TranslateMessage, MSG, WM_TIMER,
    };

    let hotkey_rx = GlobalHotKeyEvent::receiver();
    let menu_rx = MenuEvent::receiver();
    // Thread id so the settings-GUI waiter thread can wake us to reload.
    let main_tid = unsafe { GetCurrentThreadId() };

    // 1 Hz thread timer to refresh the tray tooltip with live capture stats.
    unsafe { SetTimer(None, 1, 1000, None) };

    let mut msg = MSG::default();
    loop {
        // Block until the next window/thread message.
        let got = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if got.0 == 0 {
            break; // WM_QUIT
        }
        if got.0 == -1 {
            anyhow::bail!("GetMessageW failed");
        }
        if msg.message == WM_RELOAD {
            // Settings GUI closed — reload config and re-register hotkeys live.
            reload_config_and_hotkeys(router, engine_tx);
        } else if msg.message == WM_TIMER {
            let _ = tray._icon.set_tooltip(Some(stats::tooltip()));
        } else {
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        // Drain any clip hotkeys.
        while let Ok(ev) = hotkey_rx.try_recv() {
            if ev.state == HotKeyState::Pressed {
                if let Some((seconds, name)) = router.as_ref().and_then(|r| r.resolve(ev.id)) {
                    log::info!("hotkey: save last {seconds}s ('{name}')");
                    let _ = engine_tx.send(EngineCommand::SaveClip {
                        seconds,
                        label: name.to_string(),
                    });
                }
            }
        }

        // Drain any tray menu clicks.
        while let Ok(ev) = menu_rx.try_recv() {
            if ev.id == tray.settings_id {
                launch_settings_gui(main_tid);
            } else if ev.id == tray.clips_id {
                launch_clips();
            } else if ev.id == tray.open_clips_id {
                // Ensure the clips folder actually exists before opening it,
                // so it always resolves to a real folder under Videos.
                if let Err(e) = std::fs::create_dir_all(&tray.output_dir) {
                    log::warn!(
                        "could not create clips folder {}: {e}",
                        tray.output_dir.display()
                    );
                }
                open_folder(&tray.output_dir);
            } else if ev.id == tray.quit_id {
                unsafe {
                    windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0);
                }
            }
        }
    }
    Ok(())
}

/// Reload config from disk, re-register hotkeys (old ones dropped first so the
/// same keys don't collide), and push the new config to the engine.
fn reload_config_and_hotkeys(
    router: &mut Option<HotkeyRouter>,
    engine_tx: &std::sync::mpsc::Sender<EngineCommand>,
) {
    log::info!("reloading settings");
    // Drop old registrations first, freeing the key combos.
    *router = None;
    match Config::load_or_create() {
        Ok(cfg) => {
            std::fs::create_dir_all(&cfg.output_dir).ok();
            match HotkeyRouter::register(&cfg) {
                Ok(r) => *router = Some(r),
                Err(e) => log::error!("re-registering hotkeys failed: {e}"),
            }
            let _ = engine_tx.send(EngineCommand::ReloadConfig(Box::new(cfg)));
        }
        Err(e) => log::warn!("reload failed: {e}"),
    }
}

/// Launch the settings GUI as a separate process, and when it closes, wake the
/// message loop (via a thread message) to reload config + hotkeys.
fn launch_settings_gui(main_tid: u32) {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;

    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            log::warn!("cannot locate own exe for settings GUI: {e}");
            return;
        }
    };
    match std::process::Command::new(exe).arg("--gui").spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
                unsafe {
                    let _ = PostThreadMessageW(main_tid, WM_RELOAD, WPARAM(0), LPARAM(0));
                }
            });
        }
        Err(e) => log::warn!("could not launch settings GUI: {e}"),
    }
}

/// Launch the clip library as a separate process (no config reload needed).
fn launch_clips() {
    match std::env::current_exe() {
        Ok(exe) => {
            if let Err(e) = std::process::Command::new(exe).arg("--clips").spawn() {
                log::warn!("could not launch clip library: {e}");
            }
        }
        Err(e) => log::warn!("cannot locate own exe for clip library: {e}"),
    }
}

/// Open a folder in Explorer. Pass an absolute path — Explorer does NOT
/// expand environment variables like `%APPDATA%` from the command line.
fn open_folder(path: &std::path::Path) {
    // `explorer` returns nonzero even on success for folders; ignore status.
    let _ = std::process::Command::new("explorer").arg(path).spawn();
}

#[cfg(windows)]
fn show_error_box(msg: &str) {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(msg),
            &HSTRING::from("LowResourceCapture"),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn init_logging(truncate: bool) {
    // Log to `<install>\logs\` with per-line flushing + crash capture so a
    // silent COM/Direct3D crash still leaves a complete trail. Fall back to a
    // temp dir if the install dir can't be resolved (shouldn't happen).
    let path = config::log_path()
        .unwrap_or_else(|_| std::env::temp_dir().join("lowresourcecapture.log"));
    logging::init(&path, truncate);
}
