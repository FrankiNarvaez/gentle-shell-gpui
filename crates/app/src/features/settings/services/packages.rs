//! Blocking package-manager command, using the same launcher and home
//! resolution as the terminal. UI progress and errors stay in packages.rs.

use std::path::Path;

pub(super) fn run(cwd: &Path, verb: &'static str, source: &str) -> Result<String, String> {
    crate::features::setup::run_gentle_shell(cwd, &[verb, source])
}
