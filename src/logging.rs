//! Crash-proof logging + crash capture.
//!
//! Two goals, both aimed at debugging a process that dies with no Rust error:
//!   1. **Flush every record to disk immediately.** A buffered logger loses its
//!      last lines when the process is killed by an access violation, so the
//!      log lies about where the crash happened. This logger `flush()`es after
//!      every line, so the last line written is the last thing that ran.
//!   2. **Capture hard crashes.** A Rust panic hook logs panics with location;
//!      a Win32 top-level exception filter logs access violations (the kind
//!      thrown by bad COM/Direct3D calls) with their code, address, and thread
//!      before the process goes down.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use log::{Level, LevelFilter, Log, Metadata, Record};
use windows::Win32::System::Threading::GetCurrentThreadId;

struct FileLogger {
    out: Mutex<Box<dyn Write + Send>>,
    start: Instant,
}

impl Log for FileLogger {
    fn enabled(&self, meta: &Metadata) -> bool {
        meta.level() <= Level::Info
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let e = self.start.elapsed();
        let (h, m, s, ms) = (
            e.as_secs() / 3600,
            (e.as_secs() % 3600) / 60,
            e.as_secs() % 60,
            e.subsec_millis(),
        );
        let tid = unsafe { GetCurrentThreadId() };
        if let Ok(mut out) = self.out.lock() {
            let _ = writeln!(
                out,
                "[{:02}:{:02}:{:02}.{:03}] ({:x}) {:<6} {}",
                h,
                m,
                s,
                ms,
                tid,
                record.level(),
                record.args()
            );
            // The whole point: get this line onto disk before the next call
            // (which might be the one that crashes) can run.
            let _ = out.flush();
        }
    }

    fn flush(&self) {
        if let Ok(mut out) = self.out.lock() {
            let _ = out.flush();
        }
    }
}

/// Initialize logging to `log_path` (creating its parent dir). `truncate`
/// starts a fresh file (the main process); the settings-GUI subprocess passes
/// `false` so it appends instead of wiping the main log. Falls back to stderr
/// if the file can't be opened. Also installs the panic + crash handlers.
pub fn init(log_path: &Path, truncate: bool) {
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Start a fresh file for the main process, then ALWAYS open in append mode.
    // Append is what makes multi-process logging work: the window subprocesses
    // (`--gui`, `--clips`) write to the same file, and every write goes to the
    // true end of file. Opening the main process in plain write mode instead
    // would keep its own offset, so the recorder's once-a-second stat lines
    // would silently overwrite whatever a subprocess had appended — which is
    // how clip-library lines went missing from the logs entirely.
    if truncate {
        let _ = std::fs::File::create(log_path);
    }
    let sink: Box<dyn Write + Send> = match OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
        Ok(f) => Box::new(f),
        Err(_) => Box::new(std::io::stderr()),
    };

    let logger = FileLogger {
        out: Mutex::new(sink),
        start: Instant::now(),
    };

    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(LevelFilter::Info);
    }

    install_panic_hook();
    install_crash_filter();
}

/// Log Rust panics (message + location) and flush before the default hook
/// aborts the process (we build with `panic = "abort"`).
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let tid = unsafe { GetCurrentThreadId() };
        log::error!("PANIC on thread {tid:x}: {info}");
        log::logger().flush();
    }));
}

/// Win32 top-level exception filter: logs an access violation (or other
/// structured exception) with its code + faulting address + thread, flushes,
/// then lets the OS finish tearing the process down. This is what turns a
/// silent COM/Direct3D crash into a log line pointing at the culprit thread.
fn install_crash_filter() {
    use windows::Win32::System::Diagnostics::Debug::{
        SetUnhandledExceptionFilter, EXCEPTION_POINTERS,
    };

    unsafe extern "system" fn filter(info: *const EXCEPTION_POINTERS) -> i32 {
        let tid = GetCurrentThreadId();
        let (code, addr) = if let Some(p) = info.as_ref() {
            if let Some(rec) = p.ExceptionRecord.as_ref() {
                (rec.ExceptionCode.0 as u32, rec.ExceptionAddress as usize)
            } else {
                (0, 0)
            }
        } else {
            (0, 0)
        };
        log::error!(
            "UNHANDLED EXCEPTION on thread {tid:x}: code=0x{code:08X} \
             (0xC0000005 = access violation) faulting_addr=0x{addr:X}"
        );
        log::logger().flush();
        // EXCEPTION_CONTINUE_SEARCH: let the default handler finish the crash,
        // so behavior is otherwise unchanged — we only added a log line.
        0
    }

    unsafe {
        SetUnhandledExceptionFilter(Some(filter));
    }
}
