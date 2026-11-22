//! `gentle-screen`: agents launching desktop apps, looking at their windows
//! and using them, as `gentle-browser` does for pages. The shim runs
//! `gentle-shell-gpui --screen …`; this process does the work through the
//! OS (no running app needed) and prints a JSON answer. The ops and their
//! rules are `gentle_core::screen`; `cli` runs them; `native` is the OS:
//!
//! - `windows()`: on-screen app windows, frontmost first, in the units input
//!   uses (points on macOS, pixels on Windows and Linux), top-left origin.
//! - `capture_window(&Window)` / `capture_screen()`: PNG bytes, any scale
//!   (`cli` scales a window's to its size, so its pixels are click points).
//! - `parent(pid)`: a process's parent, for `may_drive`; `alive(pid)`.
//! - `input_ready()`: Ok, or why input can't be sent (a permission).
//! - `focus(&Window)`, then `click`, `type_text`, `key`, `scroll` with
//!   window-relative coordinates.

pub mod cli;

/// The `screen` tool every agent loads (`gentle-screen.ts`), written into
/// the app's extensions folder with this executable's path in it.
pub fn install_extension() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let source = include_str!("gentle-screen.ts").replace(
        "__GENTLE_APP__",
        &serde_json::Value::from(exe.to_string_lossy()).to_string(),
    );
    let path = crate::shared::data_dir::app()?
        .join("extensions")
        .join("gentle-screen.ts");
    crate::features::genui::write_if_changed(&path, &source)?;
    Some(path)
}

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod native;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod native;
#[cfg(windows)]
#[path = "windows.rs"]
mod native;
