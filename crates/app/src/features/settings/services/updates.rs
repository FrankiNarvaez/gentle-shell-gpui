//! Blocking upgrade commands, separate from GPUI task orchestration.
//!
//! These preserve the existing launch resolution, process arguments, version
//! verification, and Engram output collection without taking ownership of
//! upgrade state or events.

use std::path::{Path, PathBuf};

/// Install and verify the Gentle Shell version the app will run afterwards.
/// The tuple is (process success, stdout+stderr, resolved version, launcher).
pub(in crate::features::settings) fn upgrade_shell(
    cwd: &Path,
    args: &[&str],
) -> (bool, String, Option<String>, Option<PathBuf>) {
    let spec = match crate::features::backend::shell_spec(cwd) {
        Ok(spec) => spec,
        Err(error) => return (false, format!("{error:#}"), None, None),
    };
    let mut command = spec.launcher_command(std::iter::once("upgrade").chain(args.iter().copied()));
    command.stdin(std::process::Stdio::null());
    let (succeeded, text) = match command.output() {
        Ok(out) => (
            out.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        ),
        Err(error) => (false, error.to_string()),
    };
    // The upgrade can move the launcher: resolve it again before reporting
    // success, just as Configuration did before the extraction.
    pi_rpc::forget_cached();
    crate::features::backend::forget_paths();
    let now = crate::features::backend::shell_spec(cwd).ok();
    let running = now.as_ref().and_then(pi_rpc::backend_version);
    (succeeded, text, running, now.map(|spec| spec.launcher))
}

/// Update Engram using its existing installer, capturing command output.
pub(in crate::features::settings) fn upgrade_engram() -> (Result<String, String>, Vec<String>) {
    let lines = std::sync::Mutex::new(Vec::new());
    let result = crate::features::setup::upgrade_engram(&|line| {
        lines.lock().unwrap().push(line);
    });
    (result, lines.into_inner().unwrap())
}
