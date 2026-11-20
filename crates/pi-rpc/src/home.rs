//! Where Gentle Shell keeps its state, resolved the way the launcher does
//! (gentle-pi `lib/gentle-shell-launcher.ts` `resolveHome`), plus the Pi
//! package the launcher runs.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::file_cache::Stamp;
use crate::launch::{LaunchSpec, extra_args, launcher_script};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Homes {
    /// The Pi agent dir: `auth.json`, `models.json`, `settings.json`, sessions.
    pub agent: PathBuf,
    /// Gentle AI config: `profiles.json`, `permission-mode.json`, …
    pub config: PathBuf,
}

pub(crate) fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn user_pi_home() -> PathBuf {
    std::env::var_os("GENTLE_SHELL_USER_PI_HOME")
        .or_else(|| std::env::var_os("PI_CODING_AGENT_DIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".pi").join("agent"))
}

/// The launcher's `config.json`, where `gentle-shell home` persists the mode:
/// `GENTLE_SHELL_CONFIG` when set and non-empty, like the launcher's own
/// `resolveConfigPath`, else `~/.gentle-shell/config.json`. gentle-ai's
/// managed install sets its own (`<root>\config\gentle-shell.json`).
pub fn launcher_config_path() -> PathBuf {
    match crate::launch::chosen_managed_install() {
        Some(managed) => managed.config_file(),
        None => config_path_in(std::env::var_os("GENTLE_SHELL_CONFIG"), &home_dir()),
    }
}

fn config_path_in(over: Option<std::ffi::OsString>, home: &Path) -> PathBuf {
    match over {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => home.join(".gentle-shell").join("config.json"),
    }
}

/// The last resolution and the config file stamp it read; everything else
/// it depends on is this process's environment.
static HOMES: Mutex<Option<(Option<Stamp>, Homes)>> = Mutex::new(None);

/// The homes of the backend agents run on (a WSL distro's, as this computer
/// reaches them), when the app runs them there: see [`use_backend_homes`].
static BACKEND_HOMES: Mutex<Option<Homes>> = Mutex::new(None);

/// Read and write the backend's homes instead of this computer's (`None`:
/// this computer's again). The app sets it when agents run in a WSL distro
/// whose home this computer reaches through `\\wsl.localhost`.
pub fn use_backend_homes(homes: Option<Homes>) {
    *BACKEND_HOMES.lock().unwrap_or_else(|e| e.into_inner()) = homes;
}

/// The homes for an agent dir in a backend: an isolated
/// `.gentle-shell/agent` keeps its config beside it (`.gentle-shell/gentle-ai`),
/// any other next to the agent dir (`~/.pi/agent` → `~/.pi/gentle-ai`).
pub fn backend_homes(agent: PathBuf) -> Homes {
    let config = agent.parent().map_or_else(
        || agent.join("gentle-ai"),
        |parent| parent.join("gentle-ai"),
    );
    Homes { agent, config }
}

/// Order: the backend's homes when agents run there ([`use_backend_homes`]),
/// gentle-ai's managed install (its own homes, whatever the app
/// inherited), then `--link`, `--isolated`, `--home <dir>` (from
/// `GENTLE_SHELL_ARGS`), then `~/.gentle-shell/config.json`, then the
/// isolated default. Cached until that config file changes.
pub fn resolve_homes() -> Homes {
    if let Some(homes) = BACKEND_HOMES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
    {
        return homes;
    }
    memo_by_stamp(&HOMES, &launcher_config_path(), resolve_homes_uncached)
}

/// `slot`'s value while `path` keeps the stamp it had when `compute` last ran.
fn memo_by_stamp<T: Clone>(
    slot: &Mutex<Option<(Option<Stamp>, T)>>,
    path: &Path,
    compute: impl FnOnce() -> T,
) -> T {
    let stamp = Stamp::of(path);
    let mut cached = slot.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, value)) = cached.as_ref()
        && *at == stamp
    {
        return value.clone();
    }
    let value = compute();
    *cached = Some((stamp, value.clone()));
    value
}

fn resolve_homes_uncached() -> Homes {
    if let Some(managed) = crate::launch::chosen_managed_install() {
        return managed.homes();
    }
    let args = extra_args();
    let isolated = || {
        std::env::var_os("GENTLE_SHELL_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir().join(".gentle-shell").join("agent"))
    };
    // Only the isolated mode keeps a Gentle Shell config folder of its own.
    let (agent, is_isolated) = if args.iter().any(|a| a == "--link") {
        (user_pi_home(), false)
    } else if args.iter().any(|a| a == "--isolated") {
        (isolated(), true)
    } else if let Some(i) = args.iter().position(|a| a == "--home")
        && let Some(dir) = args.get(i + 1)
    {
        (PathBuf::from(dir), false)
    } else {
        match read_config_mode() {
            Some(ConfigMode::Link) => (user_pi_home(), false),
            Some(ConfigMode::Path(dir)) => (dir, false),
            Some(ConfigMode::Isolated) | None => (isolated(), true),
        }
    };
    Homes {
        config: config_home(&agent, is_isolated),
        agent,
    }
}

/// gentle-pi `resolveGentlePiConfigHome` / `standaloneConfigHome`:
/// `GENTLE_PI_CONFIG_HOME` wins; a self-contained install (the launcher the
/// app runs, [`crate::launch::bundled_launcher`]) keeps it in
/// `<prefix>/config/gentle-ai` (`bundledPiConfigDir`); an isolated
/// `~/.gentle-shell/agent` keeps its config beside it; `--link` and custom
/// homes share `~/.pi/gentle-ai`.
fn config_home(agent: &Path, isolated: bool) -> PathBuf {
    if let Some(dir) = std::env::var_os("GENTLE_PI_CONFIG_HOME").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    let bundled = std::env::var_os("GENTLE_SHELL_BIN")
        .is_none_or(|bin| bin.is_empty())
        .then(crate::launch::bundled_launcher)
        .flatten();
    if let Some(dir) = bundled.as_deref().and_then(bundled_config_home) {
        return dir;
    }
    let parent = agent.parent();
    let beside = agent.file_name().is_some_and(|n| n == "agent")
        && parent
            .and_then(Path::file_name)
            .is_some_and(|n| n == ".gentle-shell");
    match parent {
        Some(parent) if isolated && beside => parent.join("gentle-ai"),
        _ => home_dir().join(".pi").join("gentle-ai"),
    }
}

/// `<prefix>/config/gentle-ai` for a self-contained install's launcher at
/// `<prefix>/bin/gentle-shell[.cmd]`.
fn bundled_config_home(launcher: &Path) -> Option<PathBuf> {
    let prefix = launcher.parent()?.parent()?;
    Some(prefix.join("config").join("gentle-ai"))
}

/// The marker a finished adoption leaves in the new config folder.
const ADOPTED_MARKER: &str = ".adopted-standalone-config";

/// Gentle Shell's self-contained install (4.7) keeps its config in
/// `<prefix>/config/gentle-ai` and never reads the folder an older launcher
/// kept beside `~/.gentle-shell/agent` (`~/.gentle-shell/gentle-ai`): after
/// upgrading, profiles, model routing and permission defaults silently stop
/// applying. Copy over once what the new folder lacks; nothing is overwritten
/// and the old folder is left as it was. Returns what was copied.
pub fn adopt_standalone_config() -> std::io::Result<Vec<PathBuf>> {
    let homes = resolve_homes();
    let legacy = homes.agent.parent().map(|parent| parent.join("gentle-ai"));
    match legacy {
        Some(legacy) if legacy != homes.config && legacy.is_dir() => {
            adopt_into(&legacy, &homes.config)
        }
        _ => Ok(Vec::new()),
    }
}

/// Copy into `target` every file of `legacy` it doesn't have yet, recursively
/// (lock and temporary files skipped), once: a marker in `target` keeps a
/// file the user later deletes from coming back.
fn adopt_into(legacy: &Path, target: &Path) -> std::io::Result<Vec<PathBuf>> {
    if target.join(ADOPTED_MARKER).exists() {
        return Ok(Vec::new());
    }
    let mut copied = Vec::new();
    copy_missing(legacy, target, &mut copied)?;
    std::fs::write(
        target.join(ADOPTED_MARKER),
        format!("copied from {}\n", legacy.display()),
    )?;
    Ok(copied)
}

fn copy_missing(from: &Path, to: &Path, copied: &mut Vec<PathBuf>) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        let text = name.to_string_lossy();
        if text.ends_with(".lock") || text.ends_with(".tmp") || text == ADOPTED_MARKER {
            continue;
        }
        let (source, dest) = (entry.path(), to.join(&name));
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_missing(&source, &dest, copied)?;
        } else if kind.is_file() && !dest.exists() {
            // `fs::copy` keeps the permission bits (0600 policies stay private).
            std::fs::copy(&source, &dest)?;
            copied.push(dest);
        }
    }
    Ok(())
}

enum ConfigMode {
    Link,
    Isolated,
    Path(PathBuf),
}

fn read_config_mode() -> Option<ConfigMode> {
    let raw = std::fs::read_to_string(launcher_config_path()).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    match value["mode"].as_str()? {
        "link" => Some(ConfigMode::Link),
        "isolated" => Some(ConfigMode::Isolated),
        "path" => value["dir"]
            .as_str()
            .map(|d| ConfigMode::Path(PathBuf::from(d))),
        _ => None,
    }
}

/// The `gentle-pi` package a launch runs: a self-contained or managed
/// install's, else the one the script its launcher runs belongs to
/// ([`launcher_script`]), through any link or package manager shim (npm,
/// pnpm, yarn).
pub fn gentle_pi_root(spec: &LaunchSpec) -> Option<PathBuf> {
    locate(spec).root
}

/// How far [`gentle_pi_root`] got: the script the launcher runs (`None` for
/// a self-contained or managed install, which names its package) and the
/// package.
struct Located {
    script: Option<PathBuf>,
    root: Option<PathBuf>,
}

fn locate(spec: &LaunchSpec) -> Located {
    if let Some(bundled) = &spec.bundled {
        return Located {
            script: None,
            root: Some(bundled.package.clone()),
        };
    }
    let script = launcher_script(&spec.launcher);
    let root = script.as_deref().and_then(gentle_pi_package_of);
    Located { script, root }
}

/// The package `script` belongs to, the nearest folder above it with a
/// `package.json` (as Node scopes it), when that package is `gentle-pi`.
fn gentle_pi_package_of(script: &Path) -> Option<PathBuf> {
    let root = script
        .ancestors()
        .skip(1)
        .find(|dir| dir.join("package.json").is_file())?;
    let manifest = std::fs::read(root.join("package.json")).ok()?;
    let manifest: serde_json::Value = serde_json::from_slice(&manifest).ok()?;
    (manifest.get("name").and_then(serde_json::Value::as_str) == Some("gentle-pi"))
        .then(|| root.to_path_buf())
}

/// Where Node looks for `@earendil-works/pi-coding-agent` from `gentle-pi`:
/// `node_modules` in the package and in each folder above it that is not a
/// `node_modules` itself. That covers nested (npm), hoisted (npm, the
/// self-contained install) and pnpm's store, where a package's dependencies
/// are its siblings (`links/@/gentle-pi/<v>/<hash>/node_modules/`).
fn pi_candidates(root: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    root.ancestors()
        .filter(|dir| dir.file_name().is_none_or(|name| name != "node_modules"))
        .map(|dir| {
            dir.join("node_modules")
                .join("@earendil-works")
                .join("pi-coding-agent")
        })
}

/// The `@earendil-works/pi-coding-agent` package `gentle-pi` runs
/// ([`gentle_pi_root`], [`pi_candidates`]). The error names the launcher and
/// what was tried.
pub fn pi_package(spec: &LaunchSpec) -> anyhow::Result<PathBuf> {
    let launcher = spec.launcher.display();
    let located = locate(spec);
    let Some(root) = located.root else {
        return Err(match located.script {
            Some(script) => anyhow::anyhow!(
                "could not locate the Pi package: the launcher {launcher} runs {}, \
                 which is not in a gentle-pi package",
                script.display()
            ),
            None => anyhow::anyhow!(
                "could not locate the Pi package: the launcher {launcher} is neither a \
                 link to gentle-pi's bin/gentle-shell.mjs nor an npm, pnpm or yarn shim \
                 that runs it"
            ),
        });
    };
    let mut tried = String::new();
    for candidate in pi_candidates(&root) {
        if candidate.join("dist").join("index.js").is_file() {
            return Ok(candidate);
        }
        tried.push_str("\n  ");
        tried.push_str(&candidate.to_string_lossy());
    }
    Err(anyhow::anyhow!(
        "could not locate the Pi package (@earendil-works/pi-coding-agent) for the \
         launcher {launcher}, gentle-pi at {}; looked in:{tried}",
        root.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::time::{Duration, SystemTime};

    #[test]
    fn standalone_config_is_adopted_once_without_overwriting() {
        let root = std::env::temp_dir().join(format!("gs-adopt-{}", std::process::id()));
        let (legacy, target) = (root.join("old"), root.join("new"));
        std::fs::create_dir_all(legacy.join("backups")).unwrap();
        std::fs::write(legacy.join("profiles.json"), "old profiles").unwrap();
        std::fs::write(legacy.join("star-prompt.json"), "old star").unwrap();
        std::fs::write(legacy.join("state.json.lock"), "").unwrap();
        std::fs::write(legacy.join("backups").join("a.json"), "b").unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("star-prompt.json"), "new star").unwrap();

        let copied = adopt_into(&legacy, &target).unwrap();
        assert_eq!(copied.len(), 2, "{copied:?}");
        assert_eq!(
            std::fs::read_to_string(target.join("profiles.json")).unwrap(),
            "old profiles"
        );
        // What the new folder already had wins; locks never move.
        assert_eq!(
            std::fs::read_to_string(target.join("star-prompt.json")).unwrap(),
            "new star"
        );
        assert!(!target.join("state.json.lock").exists());
        assert!(target.join("backups").join("a.json").exists());
        // Only once: a file deleted afterwards stays deleted.
        std::fs::remove_file(target.join("profiles.json")).unwrap();
        assert!(adopt_into(&legacy, &target).unwrap().is_empty());
        assert!(!target.join("profiles.json").exists());
        // The old folder is untouched.
        assert!(legacy.join("profiles.json").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn launcher_config_follows_gentle_shell_config_like_the_launcher() {
        let home = Path::new("/home/me");
        let default = PathBuf::from("/home/me/.gentle-shell/config.json");
        assert_eq!(config_path_in(None, home), default);
        // An empty override is ignored, as `resolveConfigPath` does.
        assert_eq!(config_path_in(Some("".into()), home), default);
        assert_eq!(
            config_path_in(Some("/tmp/other/config.json".into()), home),
            PathBuf::from("/tmp/other/config.json")
        );
    }

    #[test]
    fn bundled_install_keeps_its_config_under_its_prefix() {
        assert_eq!(
            bundled_config_home(Path::new("/home/me/.gentle-shell/bin/gentle-shell")),
            Some(PathBuf::from("/home/me/.gentle-shell/config/gentle-ai"))
        );
        assert_eq!(
            bundled_config_home(Path::new("/Local/gentle-shell/bin/gentle-shell.cmd")),
            Some(PathBuf::from("/Local/gentle-shell/config/gentle-ai"))
        );
    }

    fn set_mtime(path: &Path, secs: u64) {
        let file = File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
            .unwrap();
    }

    #[test]
    fn memo_recomputes_only_when_the_config_changes() {
        let path = std::env::temp_dir().join(format!("pi-rpc-homes-{}.json", std::process::id()));
        let slot = Mutex::new(None);
        let read = || std::fs::read_to_string(&path).ok();
        assert_eq!(memo_by_stamp(&slot, &path, read), None);

        std::fs::write(&path, "link").unwrap();
        set_mtime(&path, 2_000_000);
        assert_eq!(memo_by_stamp(&slot, &path, read).as_deref(), Some("link"));

        // Same mtime and length: the cached resolution stands.
        std::fs::write(&path, "path").unwrap();
        set_mtime(&path, 2_000_000);
        assert_eq!(memo_by_stamp(&slot, &path, read).as_deref(), Some("link"));

        // `gentle-shell home` rewrote it.
        set_mtime(&path, 2_000_001);
        assert_eq!(memo_by_stamp(&slot, &path, read).as_deref(), Some("path"));

        std::fs::remove_file(&path).unwrap();
        assert_eq!(memo_by_stamp(&slot, &path, read), None);
    }

    /// A self-contained install as Gentle Shell's installer lays it out
    /// (`scripts/bundled-install.mjs` and its tests), POSIX or Windows.
    fn bundled_prefix(windows: bool) -> (PathBuf, LaunchSpec) {
        let kind = if windows { "win" } else { "posix" };
        let prefix =
            std::env::temp_dir().join(format!("pi-rpc-bundled-{kind}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&prefix);
        let (launcher, script, node, version) = if windows {
            (
                "bin/gentle-shell.cmd",
                "@echo off\r\nrem gentle-shell bundled launcher (generated; do not edit)\r\n\
                 \"%~dp0..\\runtime\\node-24.21.0\\node.exe\" \"%~dp0..\\versions\\%GENTLE_SHELL_CURRENT%\\node_modules\\gentle-pi\\bin\\gentle-shell.mjs\" %*\r\n",
                "runtime/node-24.21.0/node.exe",
                "versions/4.7.0-1.5.0",
            )
        } else {
            (
                "bin/gentle-shell",
                "#!/bin/sh\n# gentle-shell bundled launcher (generated; do not edit)\n\
                 exec '/p/runtime/node-24.21.0/bin/node' '/p/current/node_modules/gentle-pi/bin/gentle-shell.mjs' \"$@\"\n",
                "runtime/node-24.21.0/bin/node",
                "current",
            )
        };
        let write = |path: &str, text: &str| {
            let path = prefix.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(launcher, script);
        write(node, "");
        write(
            &format!("{version}/node_modules/gentle-pi/package.json"),
            "{}",
        );
        write(
            &format!("{version}/node_modules/@earendil-works/pi-coding-agent/dist/index.js"),
            "",
        );
        if windows {
            // Windows: `current` is a file naming the active version.
            write("current", "4.7.0-1.5.0\r\n");
        }
        let program = prefix.join(launcher);
        let spec = LaunchSpec {
            bundled: crate::launch::bundled_install(&program),
            launcher: program.clone(),
            program,
            args: Vec::new(),
            cwd: prefix.clone(),
            env: Vec::new(),
        };
        (prefix, spec)
    }

    #[test]
    fn self_contained_installs_run_their_own_node_and_pi() {
        for (windows, node, version) in [
            (false, "runtime/node-24.21.0/bin/node", "current"),
            (
                true,
                "runtime/node-24.21.0/node.exe",
                "versions/4.7.0-1.5.0",
            ),
        ] {
            let (prefix, spec) = bundled_prefix(windows);
            let bundled = spec.bundled.clone().expect("bundled install");
            assert_eq!(bundled.node, prefix.join(node));
            let pi = prefix
                .join(version)
                .join("node_modules/@earendil-works/pi-coding-agent");
            assert_eq!(found_pi(&spec), dunce::canonicalize(&pi).unwrap());
            assert_eq!(gentle_pi_root(&spec), spec.bundled.map(|b| b.package));
            std::fs::remove_dir_all(&prefix).unwrap();
        }
        // An npm launcher (no generated mark) is not a self-contained install.
        let npm = std::env::temp_dir().join(format!("pi-rpc-npm-{}", std::process::id()));
        std::fs::write(&npm, "#!/usr/bin/env node\n").unwrap();
        assert!(crate::launch::bundled_install(&npm).is_none());
        std::fs::remove_file(&npm).unwrap();
    }

    /// `@earendil-works/pi-coding-agent` for `spec`, by its real path.
    fn found_pi(spec: &LaunchSpec) -> PathBuf {
        dunce::canonicalize(pi_package(spec).unwrap()).unwrap()
    }

    /// A launch through `launcher` found on the PATH (npm, pnpm, yarn).
    fn path_spec(launcher: PathBuf) -> LaunchSpec {
        LaunchSpec {
            program: launcher.clone(),
            launcher,
            args: Vec::new(),
            cwd: std::env::temp_dir(),
            env: Vec::new(),
            bundled: None,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pi-rpc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// A `gentle-pi` package at `dir`: its manifest and launcher script.
    fn gentle_pi_at(dir: &Path) {
        write(
            &dir.join("package.json"),
            r#"{"name":"gentle-pi","bin":{"gentle-shell":"bin/gentle-shell.mjs"}}"#,
        );
        write(
            &dir.join("bin").join("gentle-shell.mjs"),
            "#!/usr/bin/env node\n",
        );
    }

    /// A built `@earendil-works/pi-coding-agent` at `dir`.
    fn pi_at(dir: &Path) {
        write(&dir.join("dist").join("index.js"), "");
    }

    /// Issue #74: the web installer's main channel runs `pnpm add -g`. On
    /// Windows the PATH has pnpm's shims (`.CMD`, `.ps1` and the POSIX one,
    /// as pnpm 11 writes them), which start `gentle-shell.mjs` in pnpm's
    /// store, where Pi is a sibling of `gentle-pi`.
    #[test]
    fn pnpm_global_shims_run_pi_from_the_store() {
        let root = scratch("pnpm-shims");
        let pnpm = root.join("pnpm");
        let slot =
            r"store\v11\links\@\gentle-pi\4.8.0-main.3dc902975591\b5d0c1b6e2f1a3d4\node_modules";
        let modules = slot
            .split('\\')
            .fold(pnpm.clone(), |path, part| path.join(part));
        gentle_pi_at(&modules.join("gentle-pi"));
        let pi = modules.join("@earendil-works").join("pi-coding-agent");
        pi_at(&pi);
        let script = format!(r"..\{slot}\gentle-pi\bin\gentle-shell.mjs");
        // `@pnpm/cmd-shim` `generateCmdShim` for a `node` script.
        let cmd = format!(
            "@SETLOCAL\r\n\
             @IF NOT DEFINED NODE_PATH (\r\n  @SET \"NODE_PATH=C:\\Users\\Ana\\AppData\\Local\\pnpm\\{slot}\"\r\n\
             ) ELSE (\r\n  @SET \"NODE_PATH=C:\\Users\\Ana\\AppData\\Local\\pnpm\\{slot};%NODE_PATH%\"\r\n)\r\n\
             @IF EXIST \"%~dp0\\node.exe\" (\r\n  \"%~dp0\\node.exe\"  \"%~dp0\\{script}\" %*\r\n\
             ) ELSE (\r\n  @SET PATHEXT=%PATHEXT:;.JS;=;%\r\n  node  \"%~dp0\\{script}\" %*\r\n)\r\n"
        );
        // `generatePwshShim`.
        let pwsh = format!(
            "#!/usr/bin/env pwsh\n$basedir=Split-Path $MyInvocation.MyCommand.Definition -Parent\n\n\
             $exe=\"\"\nif ($PSVersionTable.PSVersion -lt \"6.0\" -or $IsWindows) {{\n  $exe=\".exe\"\n}}\n\
             $ret=0\nif (Test-Path \"$basedir/node$exe\") {{\n  & \"$basedir/node$exe\"  \"$basedir/{0}\" $args\n\
             }} else {{\n  & \"node$exe\"  \"$basedir/{0}\" $args\n}}\nexit $ret\n",
            script.replace('\\', "/")
        );
        // `generateShShim`: the target quoted relative to the shim, and recorded.
        let target = modules
            .join("gentle-pi")
            .join("bin")
            .join("gentle-shell.mjs");
        let sh = format!(
            "#!/bin/sh\nbasedir_abs=$(CDPATH= cd -P -- \"$basedir\" && pwd -P) || exit $?\n\
             basedir=\"$basedir_abs\"\n\
             if [ -x \"$basedir/node\" ]; then\n  exec \"$basedir/node\"  \"$basedir_abs\"/'{0}' \"$@\"\n\
             else\n  exec node  \"$basedir_abs\"/'{0}' \"$@\"\nfi\n# cmd-shim-target={1}\n",
            script.replace('\\', "/"),
            target.to_string_lossy().replace('\\', "/")
        );
        let bin = pnpm.join("bin");
        for (name, text) in [
            ("gentle-shell.CMD", cmd),
            ("gentle-shell.ps1", pwsh),
            ("gentle-shell", sh),
        ] {
            write(&bin.join(name), &text);
            let spec = path_spec(bin.join(name));
            assert_eq!(found_pi(&spec), dunce::canonicalize(&pi).unwrap(), "{name}");
            assert_eq!(
                gentle_pi_root(&spec),
                Some(dunce::canonicalize(modules.join("gentle-pi")).unwrap()),
                "{name}"
            );
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// npm on Windows: `<prefix>\gentle-shell.cmd` (npm's `cmd-shim`) beside
    /// `<prefix>\node_modules\gentle-pi`, Pi nested in it.
    #[test]
    fn npm_global_shim_runs_its_nested_pi() {
        let prefix = scratch("npm-shim");
        let package = prefix.join("node_modules").join("gentle-pi");
        gentle_pi_at(&package);
        let pi = package
            .join("node_modules")
            .join("@earendil-works")
            .join("pi-coding-agent");
        pi_at(&pi);
        let cmd = "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\n\
                   SETLOCAL\r\nCALL :find_dp0\r\n\r\nIF EXIST \"%dp0%\\node.exe\" (\r\n  \
                   SET \"_prog=%dp0%\\node.exe\"\r\n) ELSE (\r\n  SET \"_prog=node\"\r\n  \
                   SET PATHEXT=%PATHEXT:;.JS;=;%\r\n)\r\n\r\n\
                   endLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\"  \
                   \"%dp0%\\node_modules\\gentle-pi\\bin\\gentle-shell.mjs\" %*\r\n";
        write(&prefix.join("gentle-shell.cmd"), cmd);
        let spec = path_spec(prefix.join("gentle-shell.cmd"));
        assert_eq!(found_pi(&spec), dunce::canonicalize(&pi).unwrap());
        std::fs::remove_dir_all(&prefix).unwrap();
    }

    /// macOS and Linux: npm links `<prefix>/bin/gentle-shell` to the script;
    /// pnpm's global folder links `gentle-pi` into its store, whose Pi is a
    /// link to Pi's own store entry.
    #[cfg(unix)]
    #[test]
    fn links_lead_into_the_npm_prefix_and_the_pnpm_store() {
        use std::os::unix::fs::symlink;
        let root = scratch("links");

        let lib = root.join("npm").join("lib").join("node_modules");
        gentle_pi_at(&lib.join("gentle-pi"));
        let npm_pi = lib.join("@earendil-works").join("pi-coding-agent");
        pi_at(&npm_pi);
        let npm_bin = root.join("npm").join("bin");
        std::fs::create_dir_all(&npm_bin).unwrap();
        symlink(
            "../lib/node_modules/gentle-pi/bin/gentle-shell.mjs",
            npm_bin.join("gentle-shell"),
        )
        .unwrap();
        let spec = path_spec(npm_bin.join("gentle-shell"));
        assert_eq!(found_pi(&spec), dunce::canonicalize(&npm_pi).unwrap());

        let links = root.join("pnpm").join("store").join("v11").join("links");
        let modules = links.join("@/gentle-pi/4.8.0/b5d0/node_modules");
        gentle_pi_at(&modules.join("gentle-pi"));
        let pi = links.join("@earendil-works/pi-coding-agent/1.5.2/9e1f/node_modules/@earendil-works/pi-coding-agent");
        pi_at(&pi);
        std::fs::create_dir_all(modules.join("@earendil-works")).unwrap();
        symlink(&pi, modules.join("@earendil-works").join("pi-coding-agent")).unwrap();
        let global = root.join("pnpm").join("global/v11/7c3a/node_modules");
        std::fs::create_dir_all(&global).unwrap();
        symlink(modules.join("gentle-pi"), global.join("gentle-pi")).unwrap();
        let target = global.join("gentle-pi/bin/gentle-shell.mjs");
        write(
            &root.join("pnpm").join("gentle-shell"),
            &format!(
                "#!/bin/sh\nexec node  \"$basedir_abs\"/'global/v11/7c3a/node_modules/gentle-pi/bin/gentle-shell.mjs' \"$@\"\n\
                 # cmd-shim-target={}\n",
                target.display()
            ),
        );
        // The shim itself reached through a link, as `~/.local/bin` might.
        symlink(
            root.join("pnpm").join("gentle-shell"),
            root.join("gentle-shell"),
        )
        .unwrap();
        for launcher in [
            root.join("pnpm").join("gentle-shell"),
            root.join("gentle-shell"),
        ] {
            let spec = path_spec(launcher);
            assert_eq!(found_pi(&spec), dunce::canonicalize(&pi).unwrap());
            assert_eq!(
                gentle_pi_root(&spec),
                Some(dunce::canonicalize(modules.join("gentle-pi")).unwrap())
            );
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_missing_pi_names_the_launcher_and_what_was_tried() {
        let root = scratch("missing");
        // Not a link nor a shim.
        let other = root.join("gentle-shell.cmd");
        write(&other, "@echo off\r\necho hi\r\n");
        let error = pi_package(&path_spec(other.clone()))
            .unwrap_err()
            .to_string();
        assert!(error.contains(&other.display().to_string()), "{error}");
        assert!(error.contains("nor an npm, pnpm or yarn shim"), "{error}");
        // A shim into another package.
        let elsewhere = root.join("other");
        write(&elsewhere.join("package.json"), r#"{"name":"other"}"#);
        write(&elsewhere.join("cli.js"), "");
        let shim = root.join("other.cmd");
        write(&shim, "@node \"%~dp0\\other\\cli.js\" %*\r\n");
        let error = pi_package(&path_spec(shim)).unwrap_err().to_string();
        assert!(error.contains("not in a gentle-pi package"), "{error}");
        // gentle-pi without Pi: every folder Node would look in is listed.
        let package = root.join("node_modules").join("gentle-pi");
        gentle_pi_at(&package);
        let shim = root.join("gentle-shell.ps1");
        write(
            &shim,
            "& \"node$exe\"  \"$basedir/node_modules/gentle-pi/bin/gentle-shell.mjs\" $args\n",
        );
        let error = pi_package(&path_spec(shim.clone()))
            .unwrap_err()
            .to_string();
        assert!(error.contains(&shim.display().to_string()), "{error}");
        let real = dunce::canonicalize(&root).unwrap();
        for tried in [
            "node_modules/gentle-pi/node_modules/@earendil-works/pi-coding-agent",
            "node_modules/@earendil-works/pi-coding-agent",
        ] {
            let tried = tried
                .split('/')
                .fold(real.clone(), |path, part| path.join(part));
            assert!(error.contains(&tried.display().to_string()), "{error}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }
}
