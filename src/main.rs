// LowResourceCapture — retroactive game clip recorder for Windows.
//
// Release builds hide the console window; debug builds keep it for logs.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod audio;
mod capture;
mod config;
mod encoder;
mod engine;
mod game_detect;
mod hotkeys;
mod muxer;
mod ringbuffer;

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
    init_logging();
    log::info!("LowResourceCapture starting");

    let config = Config::load_or_create()?;
    std::fs::create_dir_all(&config.output_dir).ok();

    // Start the capture engine (idle until a game is detected).
    let (engine_handle, engine_tx) = engine::spawn(config.clone());

    // Register the clip hotkeys.
    let router = HotkeyRouter::register(&config)?;

    // Build the tray icon + menu.
    let tray = build_tray(&config)?;

    log::info!("ready; sitting in the tray. Press a clip hotkey while in a game.");

    // Pump the Win32 message loop and route hotkey + menu events.
    // NOTE(first build): the exact `GetMessageW` argument types can differ by
    // one wrapper (`Option<HWND>` vs `HWND`) across windows-rs point releases —
    // this is the single most likely spot to need a one-line tweak on your
    // first `cargo build`. Everything downstream is independent of it.
    run_message_loop(&router, &engine_tx, &tray)?;

    engine_handle.shutdown();
    log::info!("exited cleanly");
    Ok(())
}

/// The tray menu items we need to compare events against.
struct Tray {
    _icon: tray_icon::TrayIcon,
    open_clips_id: tray_icon::menu::MenuId,
    open_config_id: tray_icon::menu::MenuId,
    reload_id: tray_icon::menu::MenuId,
    quit_id: tray_icon::menu::MenuId,
    output_dir: std::path::PathBuf,
    config_path_hint: String,
}

fn build_tray(config: &Config) -> Result<Tray> {
    let menu = Menu::new();
    let open_clips = MenuItem::new("Open clips folder", true, None);
    let open_config = MenuItem::new("Edit settings (config.toml)", true, None);
    let reload = MenuItem::new("Reload settings", true, None);
    let quit = MenuItem::new("Quit", true, None);

    menu.append(&open_clips)?;
    menu.append(&open_config)?;
    menu.append(&reload)?;
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
        open_clips_id: open_clips.id().clone(),
        open_config_id: open_config.id().clone(),
        reload_id: reload.id().clone(),
        quit_id: quit.id().clone(),
        output_dir: config.output_dir.clone(),
        config_path_hint: "%APPDATA%\\LowResourceCapture\\config.toml".to_string(),
    })
}

/// A minimal 32x32 icon generated in code (a red dot) so we ship no asset
/// files. Swap for a real .ico later.
fn make_tray_icon() -> tray_icon::Icon {
    const S: u32 = 32;
    let mut rgba = vec![0u8; (S * S * 4) as usize];
    let c = (S as f32 - 1.0) / 2.0;
    let r = S as f32 * 0.40;
    for y in 0..S {
        for x in 0..S {
            let dx = x as f32 - c;
            let dy = y as f32 - c;
            let inside = (dx * dx + dy * dy).sqrt() <= r;
            let i = ((y * S + x) * 4) as usize;
            if inside {
                rgba[i] = 0xE0; // R
                rgba[i + 1] = 0x2B; // G
                rgba[i + 2] = 0x2B; // B
                rgba[i + 3] = 0xFF; // A
            }
        }
    }
    tray_icon::Icon::from_rgba(rgba, S, S).expect("building tray icon")
}

fn run_message_loop(
    router: &HotkeyRouter,
    engine_tx: &std::sync::mpsc::Sender<EngineCommand>,
    tray: &Tray,
) -> Result<()> {
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, TranslateMessage, MSG,
    };

    let hotkey_rx = GlobalHotKeyEvent::receiver();
    let menu_rx = MenuEvent::receiver();

    let mut msg = MSG::default();
    loop {
        // Block until the next window message (hotkey/tray/menu all post here).
        let got = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if got.0 == 0 {
            break; // WM_QUIT
        }
        if got.0 == -1 {
            anyhow::bail!("GetMessageW failed");
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        // Drain any clip hotkeys.
        while let Ok(ev) = hotkey_rx.try_recv() {
            if ev.state == HotKeyState::Pressed {
                if let Some((seconds, name)) = router.resolve(ev.id) {
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
            if ev.id == tray.open_clips_id {
                open_path(&tray.output_dir.to_string_lossy());
            } else if ev.id == tray.open_config_id {
                match config::Config::load_or_create() {
                    Ok(_) => open_path(&tray.config_path_hint),
                    Err(e) => log::warn!("could not locate config: {e}"),
                }
            } else if ev.id == tray.reload_id {
                match Config::load_or_create() {
                    Ok(cfg) => {
                        log::info!("reloading settings");
                        let _ = engine_tx.send(EngineCommand::ReloadConfig(Box::new(cfg)));
                        // NOTE: hotkey changes take effect on next launch until
                        // layer 5 adds live re-registration.
                    }
                    Err(e) => log::warn!("reload failed: {e}"),
                }
            } else if ev.id == tray.quit_id {
                unsafe {
                    windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0);
                }
            }
        }
    }
    Ok(())
}

/// Open a folder/file with the shell (Explorer / default handler).
fn open_path(path: &str) {
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

fn init_logging() {
    // Log to a file next to the config so windowed builds still leave a trail.
    if let Some(dirs) = directories::ProjectDirs::from("", "", "LowResourceCapture") {
        let dir = dirs.data_dir();
        std::fs::create_dir_all(dir).ok();
        let log_path = dir.join("lowresourcecapture.log");
        let _ = simple_logging::log_to_file(&log_path, log::LevelFilter::Info);
    } else {
        let _ = simple_logging::log_to_stderr(log::LevelFilter::Info);
    }
}
