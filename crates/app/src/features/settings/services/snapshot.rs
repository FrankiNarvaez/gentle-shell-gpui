//! Read and enrich one configuration snapshot.
//!
//! Runs only on the background executor. Keep this in the same order as the
//! prior SettingsView::reload implementation: read files, resolve launcher,
//! probe, check Gentle Shell update, check Engram update.

use std::path::Path;

use super::super::snapshot::{Snapshot, probe, read_snapshot};

pub(in crate::features::settings) fn load(cwd: &Path, shell_updates: bool) -> Snapshot {
    // Where agents run in WSL, its homes; never this computer's by mistake
    // while they are still being asked for.
    crate::features::backend::ensure_homes();
    let homes = pi_rpc::resolve_homes();
    let mut snapshot = read_snapshot(&homes.agent, &homes.config, cwd);
    let spec = crate::features::backend::shell_spec(cwd)
        .or_else(|_| crate::features::backend::home_shell_spec())
        .ok();
    if let Some(spec) = &spec {
        probe(spec, &mut snapshot);
    }
    // In a backend the launcher is its `gentle-shell`, named with where.
    let choice = crate::features::backend::for_tab(cwd);
    if choice != gentle_core::backend::Backend::Local {
        snapshot.launcher = crate::features::backend::gentle_shell_path(&choice)
            .map(|path| format!("{path} ({})", choice.label()).into());
    }
    // gentle-ai owns updates of installs it manages; a responding Shell
    // supplies update status in preference to the launcher registry check.
    snapshot.managed = spec
        .as_ref()
        .is_some_and(pi_rpc::LaunchSpec::managed_by_gentle_ai);
    snapshot.skipped_release = crate::features::updates::skipped();
    if !snapshot.managed && !shell_updates {
        snapshot.update = crate::features::updates::current_from_versions(&snapshot.versions)
            .and_then(crate::features::updates::check);
    }
    snapshot.engram_update = crate::features::setup::engram_version()
        .and_then(|v| crate::features::updates::engram::check(&v));
    snapshot
}
