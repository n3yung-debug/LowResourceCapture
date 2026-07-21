//! Embed the Windows application icon into the exe so Explorer, the taskbar,
//! Alt-Tab, and the installer's UninstallDisplayIcon all show it. Cosmetic —
//! if the resource compiler isn't available the error is ignored and the build
//! proceeds with the default icon.

fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        let _ = res.compile();
    }
    println!("cargo:rerun-if-changed=assets/icon.ico");
}
