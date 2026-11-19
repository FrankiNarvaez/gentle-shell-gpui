//! Where agents run (`gentle_core::backend`): the choice saved in the app's
//! `settings.json`, the WSL distro a project folder implies, that
//! environment's login PATH and Gentle Shell homes, the WSL distros on this
//! computer, and what runs through it: `gentle-shell` (agent tabs and its
//! commands), terminals and `git`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{LazyLock, Mutex, RwLock};
use std::time::Duration;

use anyhow::{Result, anyhow};
use gentle_core::backend::{self, Backend, Distro};
use gpui_kit::App;
use serde_json::Value;

/// How long the backend may take to answer a probe: a stopped WSL distro
/// or VM boots first.
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// The saved choice, read once: `git` asks for it on every refresh.
static SAVED: RwLock<Option<Backend>> = RwLock::new(None);

/// Read the backend's homes, if agents run in one, in the background.
pub fn init(cx: &mut App) {
    cx.background_executor()
        .spawn(async { sync_homes() })
        .detach();
}

/// The backend new agent tabs use; [`Backend::Local`] when none is saved.
pub fn saved() -> Backend {
    if let Some(known) = SAVED.read().ok().and_then(|saved| saved.clone()) {
        return known;
    }
    let choice = crate::shared::themes::store_path()
        .and_then(|path| crate::shared::themes::read_key(&path, backend::KEY))
        .map_or_else(Backend::default, |value| Backend::from_json(&value));
    if let Ok(mut saved) = SAVED.write() {
        *saved = Some(choice.clone());
    }
    choice
}

/// Save the backend new agent tabs use (`null` for this computer).
pub fn save(choice: &Backend) -> std::io::Result<()> {
    let path = crate::shared::themes::store_path()
        .ok_or_else(|| std::io::Error::other("no data folder"))?;
    crate::shared::themes::write_key(&path, backend::KEY, choice.to_json().unwrap_or(Value::Null))?;
    if let Ok(mut saved) = SAVED.write() {
        *saved = Some(choice.clone());
    }
    Ok(())
}

/// Where work rooted at `cwd` runs: the distro of a folder under
/// `\\wsl.localhost\<distro>\`, else the saved choice.
pub fn for_tab(cwd: &Path) -> Backend {
    Backend::for_folder(&cwd.to_string_lossy()).unwrap_or_else(saved)
}

/// How `choice` reads `path`, a path on this computer, or why it can't.
fn reach(choice: &Backend, path: &Path) -> Result<String> {
    choice.path(&path.to_string_lossy()).ok_or_else(|| {
        anyhow!(
            "{} can't reach {}. Open the project from a folder it can reach.",
            choice.label(),
            path.display()
        )
    })
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
    let folder = reach(choice, cwd)?;
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
    // Files the agent shares with the app while it runs (its canvas) are read
    // in place; the backend must see them.
    let mut all = env.to_vec();
    for (key, file) in files {
        all.push((key.clone(), reach(choice, file)?));
    }
    let path = login_path(choice);
    pi_rpc::remote_backend(&launch, path.as_deref(), &all, |file| stage(choice, file))
}

/// A file the app gives every agent (its instructions, its extensions),
/// copied into `choice`, which may not see this computer's files (a
/// container): its path there, `~/.cache/gentle-shell-gpui/<hash>-<name>`.
/// Named by content, so app versions side by side never mix; copied once.
fn stage(choice: &Backend, file: &Path) -> Option<String> {
    static STAGED: LazyLock<Mutex<HashSet<(Backend, String)>>> = LazyLock::new(Mutex::default);
    let bytes = std::fs::read(file).ok()?;
    let hash = {
        use std::hash::{Hash as _, Hasher as _};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut hasher);
        hasher.finish()
    };
    let home = remote_home(choice)?;
    let name = file.file_name()?.to_string_lossy();
    let there = format!("{home}/.cache/gentle-shell-gpui/{hash:016x}-{name}");
    let key = (choice.clone(), there.clone());
    if STAGED.lock().ok()?.contains(&key) {
        return Some(there);
    }
    let mut argv = choice.launch("/")?;
    // Written beside and moved in, so a tab starting meanwhile never reads half.
    argv.extend(
        [
            "sh",
            "-c",
            r#"mkdir -p "${1%/*}" && cat > "$1.$$" && mv -f "$1.$$" "$1""#,
            "sh",
            &there,
        ]
        .map(str::to_string),
    );
    let (program, args) = argv.split_first()?;
    let mut command = pi_rpc::command(program);
    command
        .args(args)
        .stdin(std::fs::File::open(file).ok()?)
        .stderr(std::process::Stdio::null());
    let (ok, _) = pi_rpc::output_within(&mut command, PROBE_TIMEOUT)?;
    if !ok {
        return None;
    }
    STAGED.lock().ok()?.insert(key);
    Some(there)
}

/// The backend user's home folder, asked once per backend.
fn remote_home(choice: &Backend) -> Option<String> {
    static HOMES: LazyLock<Mutex<HashMap<Backend, String>>> = LazyLock::new(Mutex::default);
    if let Some(home) = HOMES.lock().ok()?.get(choice) {
        return Some(home.clone());
    }
    let (ok, home) = run_in(
        choice,
        &["sh", "-c", r#"printf %s "$HOME""#].map(str::to_string),
    )?;
    let home = home.trim().trim_end_matches('/').to_string();
    if !ok || !home.starts_with('/') {
        return None;
    }
    HOMES.lock().ok()?.insert(choice.clone(), home.clone());
    Some(home)
}

/// `gentle-shell` where `cwd`'s agents run, for its own commands in that
/// folder (`install -l`, `upgrade`): [`pi_rpc::LaunchSpec::launcher_command`]
/// runs them. Blocking, like [`launch_spec`].
pub fn shell_spec(cwd: &Path) -> Result<pi_rpc::LaunchSpec> {
    launch_spec(&for_tab(cwd), cwd, &[], &[])
}

/// `gentle-shell` where new agent tabs run, for commands about the
/// installation (`--version`, `setup`, `install`): from the user's home
/// here, or the backend's root, which every backend has (a container may not
/// see this computer's home). Blocking, like [`launch_spec`].
pub fn home_shell_spec() -> Result<pi_rpc::LaunchSpec> {
    match saved() {
        Backend::Local => {
            pi_rpc::resolve_backend(&dirs::home_dir().unwrap_or_else(std::env::temp_dir))
        }
        choice => launch_spec(&choice, Path::new("/"), &[], &[]),
    }
}

/// `git` for the repository at `cwd`, run where `cwd`'s agents run (in a
/// WSL distro instead of over `\\wsl.localhost`, which is slow), with `env`
/// set; the caller adds the arguments. A folder the backend can't reach
/// runs on this computer, the only place that can read it.
pub fn git(cwd: &Path, env: &[(&str, &str)]) -> Command {
    let choice = for_tab(cwd);
    let launch = reach(&choice, cwd)
        .ok()
        .and_then(|folder| choice.launch(&folder));
    match launch.as_deref() {
        Some([program, args @ ..]) => {
            let mut command = pi_rpc::command(program);
            command
                .args(args)
                .arg("env")
                .args(env.iter().map(|(key, value)| format!("{key}={value}")))
                .arg("git");
            command
        }
        _ => {
            let mut command = pi_rpc::command("git");
            command.current_dir(cwd).envs(env.iter().copied());
            command
        }
    }
}

/// A path `git` printed for the repository at `cwd` (its top level, a
/// worktree), as this computer reads it.
pub fn host_path(cwd: &Path, printed: &str) -> PathBuf {
    PathBuf::from(for_tab(cwd).host_path(printed))
}

/// `host`, a path on this computer, as the backend that runs work rooted at
/// `cwd` reads it (for `git worktree add <folder>`); as it is when the
/// backend can't reach it.
pub fn there(cwd: &Path, host: &Path) -> String {
    let host = host.to_string_lossy();
    for_tab(cwd)
        .path(&host)
        .unwrap_or_else(|| host.into_owned())
}

/// The argv of a terminal in `cwd` where its agents run: the backend's
/// login shell there; `None` on this computer.
pub fn terminal(cwd: &Path) -> Option<Vec<String>> {
    let choice = for_tab(cwd);
    choice.terminal(&reach(&choice, cwd).ok()?)
}

/// Read a file of the saved backend, `path` as the backend sees it: through
/// `\\wsl.localhost` for a WSL distro, with `cat` through a command.
/// Blocking.
pub fn read(choice: &Backend, path: &str) -> Option<String> {
    match choice {
        Backend::Local => std::fs::read_to_string(path).ok(),
        Backend::Wsl { .. } => std::fs::read_to_string(choice.host_path(path)).ok(),
        Backend::Command { .. } => run_in(choice, &["cat".to_string(), path.to_string()])
            .filter(|(ok, _)| *ok)
            .map(|(_, text)| text),
    }
}

/// Run `program` with `args` on `choice` with its login PATH: whether it
/// succeeded, and its output (stdout). Blocking, within the probe timeout.
pub fn run(choice: &Backend, program: &str, args: &[&str]) -> Option<(bool, String)> {
    let mut argv = vec!["env".to_string()];
    argv.extend(login_path(choice).map(|path| format!("PATH={path}")));
    argv.push(program.to_string());
    argv.extend(args.iter().map(|arg| arg.to_string()));
    run_in(choice, &argv)
}

/// Point the app at the saved backend's Gentle Shell homes ([`use_home`]).
/// Blocking: it asks `gentle-shell --version` there.
pub fn sync_homes() {
    let choice = saved();
    let home = match choice {
        Backend::Local => None,
        _ => crate::features::setup::launcher_home(),
    };
    use_home(&choice, home.as_deref());
}

/// Use `home`, the agent dir `gentle-shell --version` names in `choice`, as
/// the app's Gentle Shell home when this computer reaches it (a WSL
/// distro's, through `\\wsl.localhost`), so Configuration, sessions and
/// usage read the files the agents use; this computer's own otherwise.
pub fn use_home(choice: &Backend, home: Option<&str>) {
    let homes = matches!(choice, Backend::Wsl { .. })
        .then_some(home)
        .flatten()
        .map(|dir| PathBuf::from(choice.host_path(dir)))
        .filter(|dir| dir.is_dir())
        .map(pi_rpc::backend_homes);
    pi_rpc::use_backend_homes(homes);
}

/// Each backend's login PATH, asked once and kept.
static PATHS: LazyLock<Mutex<HashMap<Backend, Option<String>>>> = LazyLock::new(Mutex::default);

/// Ask the login PATHs again: something was installed there.
pub fn forget_paths() {
    if let Ok(mut paths) = PATHS.lock() {
        paths.clear();
    }
}

/// The backend's login PATH, asked once per backend and kept.
fn login_path(choice: &Backend) -> Option<String> {
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
    run(choice, "sh", &["-c", "command -v gentle-shell"]).is_some_and(|(ok, _)| ok)
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
