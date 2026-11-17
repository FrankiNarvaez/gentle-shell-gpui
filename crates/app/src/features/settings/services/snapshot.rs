//! Read and enrich one configuration snapshot.
//!
//! Runs only on the background executor. Keep this in the same order as the
//! prior SettingsView::reload implementation: read files, resolve launcher,
//! probe, check Gentle Shell update, check Engram update.

use std::path::Path;

use super::super::snapshot::{Snapshot, probe, read_snapshot};

pub(in crate::features::settings) fn load(cwd: &Path) -> Snapshot {
    let homes = pi_rpc::resolve_homes();
    let mut snapshot = read_snapshot(&homes.agent, &homes.config, cwd);
    let spec = pi_rpc::resolve_backend(cwd).ok();
    if let Some(spec) = &spec {
        probe(spec, &mut snapshot);
    }
    // Cached for a day in the launcher config, so reopening Configuration
    // stays off the network. gentle-ai updates managed installs itself.
    if !spec
        .as_ref()
        .is_some_and(pi_rpc::LaunchSpec::managed_by_gentle_ai)
    {
        snapshot.update =
            crate::features::updates::current_from_versions(&snapshot.versions)
                .and_then(crate::features::updates::check);
    }
    snapshot.engram_update = crate::features::setup::engram_version()
        .and_then(|v| crate::features::updates::engram::check(&v));
    snapshot
}
