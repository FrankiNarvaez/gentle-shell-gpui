//! Where agent tabs run (`gentle_core::backend`): the choice saved in the
//! app's `settings.json`, the WSL distro a project folder implies, that
//! environment's login PATH, the WSL distros on this computer, and the
//! launch of `gentle-shell --mode rpc` through it.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow};
use gentle_core::backend::{self, Backend, Distro};
use serde_json::Value;

/// How long the backend may take to answer a probe: a stopped WSL distro
/// or VM boots first.
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// The backend new agent tabs use; [`Backend::Local`] when none is saved.
pub fn saved() -> Backend {
    crate::shared::themes::store_path()
        .and_then(|path| crate::shared::themes::read_key(&path, backend::KEY))
        .map_or_else(Backend::default, |value| Backend::from_json(&value))
}

/// Save the backend new agent tabs use (`null` for this computer).
pub fn save(choice: &Backend) -> std::io::Result<()> {
    let path = crate::shared::themes::store_path()
        .ok_or_else(|| std::io::Error::other("no data folder"))?;
    crate::shared::themes::write_key(&path, backend::KEY, choice.to_json().unwrap_or(Value::Null))
}

/// Where an agent tab rooted at `cwd` runs: the distro of a folder under
/// `\\wsl.localhost\<distro>\`, else the saved choice.
pub fn for_tab(cwd: &Path) -> Backend {
    Backend::for_folder(&cwd.to_string_lossy()).unwrap_or_else(saved)
}

/// The launch of an agent tab's `gentle-shell --mode rpc` in `cwd` on
/// `choice`, with `env` set and each `files` variable naming a host file as
/// the backend reads it. Blocking: a backend's first launch asks for its
/// login PATH.
pub fn launch_spec(
    choice: &Backend,
    cwd: &Path,
    env: &[(String, String)],
    files: &[(String, &Path)],
) -> Result<pi_rpc::LaunchSpec> {
    let reach = |path: &Path| {
        choice.path(&path.to_string_lossy()).ok_or_else(|| {
            anyhow!(
                "{} can't reach {}. Open the project from a folder it can reach.",
                choice.label(),
                path.display()
            )
        })
    };
    let folder = reach(cwd)?;
    let Some(launch) = choice.launch(&folder) else {
        let mut spec = pi_rpc::resolve_backend(cwd)?;
        spec.env
            .extend(env.iter().map(|(k, v)| (k.clone(), v.into())));
        spec.env.extend(
            files
                .iter()
                .map(|(k, p)| (k.clone(), p.as_os_str().to_owned())),
        );
        return Ok(spec);
    };
    let mut all = env.to_vec();
    for (key, file) in files {
        all.push((key.clone(), reach(file)?));
    }
    let path = login_path(choice);
    pi_rpc::remote_backend(&launch, path.as_deref(), &all, |file| {
        choice.path(&file.to_string_lossy())
    })
}

/// The backend's login PATH, asked once per backend and kept.
fn login_path(choice: &Backend) -> Option<String> {
    static PATHS: LazyLock<Mutex<HashMap<Backend, Option<String>>>> = LazyLock::new(Mutex::default);
    if let Some(known) = PATHS.lock().ok()?.get(choice) {
        return known.clone();
    }
    let path =
        run_in(choice, &backend::path_probe()).and_then(|(_, out)| backend::parse_path_probe(&out));
    if let Ok(mut paths) = PATHS.lock() {
        paths.insert(choice.clone(), path.clone());
    }
    path
}

/// Run `argv` on `choice` from its root folder, within [`PROBE_TIMEOUT`]:
/// whether it succeeded, and its output.
fn run_in(choice: &Backend, argv: &[String]) -> Option<(bool, String)> {
    let mut launch = choice.launch("/")?;
    launch.extend(argv.iter().cloned());
    let (program, args) = launch.split_first()?;
    let mut command = pi_rpc::command(program);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    pi_rpc::output_within(&mut command, PROBE_TIMEOUT)
}

/// Whether `gentle-shell` is on `choice`'s login PATH.
pub fn has_gentle_shell(choice: &Backend) -> bool {
    let mut argv = vec!["env".to_string()];
    argv.extend(login_path(choice).map(|path| format!("PATH={path}")));
    argv.extend(["sh", "-c", "command -v gentle-shell"].map(str::to_string));
    run_in(choice, &argv).is_some_and(|(ok, _)| ok)
}

/// This computer's WSL distros (`wsl.exe --list --verbose`); empty without
/// WSL, and anywhere but Windows.
pub fn wsl_distros() -> Vec<Distro> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let mut command = pi_rpc::command("wsl.exe");
    command
        .args(["--list", "--verbose"])
        // UTF-8 output on current WSL; `parse_distros` copes with UTF-16 too.
        .env("WSL_UTF8", "1")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    pi_rpc::output_within(&mut command, PROBE_TIMEOUT)
        .filter(|(ok, _)| *ok)
        .map(|(_, out)| backend::parse_distros(&out))
        .unwrap_or_default()
}
