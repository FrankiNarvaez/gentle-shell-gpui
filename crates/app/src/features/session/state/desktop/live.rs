//! Live check of the review-mode, changes and config slices against a real
//! Gentle Shell: the tab's own [`Desktop`] state (probe, slices, shared
//! session) driven by a real `gentle-shell --mode rpc` in a temporary home
//! and Git repository, routed the way the tab routes records.
//!
//! `#[ignore]` by default. To run it against Gentle Shell `main` (or 4.8.0,
//! where both features must stay hidden and nothing new is ever sent):
//!
//! ```text
//! GENTLE_SHELL_LIVE_LAUNCHER=.../bin/gentle-shell.mjs \
//!   cargo test -p gentle-shell-gpui --locked live_review_changes -- --ignored --nocapture
//! ```
//!
//! The review round needs the `gentle-ai` binary the launcher resolves (on a
//! source checkout, `<repo>/.gentle-ai/v*/gentle-ai`); without it the switch
//! answers `review_unavailable` and the chip stays hidden, which is checked
//! instead. No credentials, no network (`PI_OFFLINE=1`), no auto setup.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pi_rpc::desktop_rpc::{
    ChangeStatus, ConfigStore, DesktopMethod, DesktopRecord, ReviewEffective, ReviewScope,
    ReviewSetting, ReviewSource, decode_widget,
};
use pi_rpc::{Incoming, LaunchSpec, PiProcess, UiRequest};
use serde_json::json;

use super::changes::{FileDiff, FileKey};
use super::{Desktop, Send, Step, is_desktop_widget};

const STEP: Duration = Duration::from_secs(60);

struct Live {
    process: Arc<PiProcess>,
    incoming: std::sync::mpsc::Receiver<Incoming>,
    desktop: Desktop,
    root: PathBuf,
}

impl Drop for Live {
    fn drop(&mut self) {
        self.process.shutdown(Duration::from_secs(5));
        let deadline = Instant::now() + Duration::from_secs(15);
        while let Ok(item) = self
            .incoming
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            if matches!(item, Incoming::Exited(_)) {
                break;
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Live {
    /// Hand each request to the backend and require the prompt `handled`,
    /// as `send_one_desktop` does.
    fn send(&mut self, sends: Vec<Send>) {
        for send in sends {
            let outer = smol::block_on(self.process.request(send.request.prompt_command()))
                .expect("outer prompt");
            assert_eq!(outer.data["disposition"], "handled");
        }
    }

    /// Route one Desktop widget like `handle_desktop_ui`: by the pending
    /// method for answers, by name for the events these slices own.
    fn route(&mut self, request: &UiRequest) {
        let decoded = decode_widget(request);
        if super::changes::changes_moved(&decoded) {
            self.desktop.changes_mark_stale();
        }
        let pending = self.desktop.session().pending().map(|(_, method)| method);
        let sends = match (&decoded, pending) {
            (Ok(Some(DesktopRecord::Response(_))), Some(method)) => match method {
                DesktopMethod::GetReviewMode | DesktopMethod::SetReviewMode => {
                    self.desktop.review_apply(request).sends
                }
                DesktopMethod::GetChanges | DesktopMethod::GetChangeDiff => {
                    self.desktop.changes_apply(request).sends
                }
                DesktopMethod::GetConfig | DesktopMethod::SetConfig => {
                    self.desktop.config_apply(request).sends
                }
                _ => self.desktop.ui(request).sends,
            },
            (Ok(Some(DesktopRecord::Event(event))), _) if event.event == "review_mode_changed" => {
                self.desktop.review_apply(request).sends
            }
            (Ok(Some(DesktopRecord::Event(event))), _) if event.event == "config_changed" => {
                self.desktop.config_apply(request).sends
            }
            _ => self.desktop.ui(request).sends,
        };
        self.send(sends);
        self.pump();
    }

    /// Pump records until `done` holds or the step times out.
    fn until(&mut self, what: &str, within: Duration, done: impl Fn(&Desktop) -> bool) -> bool {
        let deadline = Instant::now() + within;
        while !done(&self.desktop) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                eprintln!("live: gave up waiting for {what}");
                return false;
            }
            match self.incoming.recv_timeout(remaining) {
                Ok(Incoming::Ui(request)) if is_desktop_widget(&request) => self.route(&request),
                Ok(Incoming::Exited(code)) => panic!("backend exited ({code:?})"),
                Ok(Incoming::Stderr(line)) => eprintln!("[backend] {line}"),
                Ok(_) => {}
                Err(_) => {
                    eprintln!("live: gave up waiting for {what}");
                    return false;
                }
            }
        }
        true
    }

    fn pump(&mut self) {
        let review = self.desktop.review_pump().sends;
        self.send(review);
        let changes = self.desktop.changes_pump().sends;
        self.send(changes);
    }
}

fn find_node() -> Option<PathBuf> {
    let name = if cfg!(windows) { "node.exe" } else { "node" };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn git(workspace: &Path, env: &[(String, OsString)], args: &[&str]) {
    let status = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=live",
            "-c",
            "user.email=live@example.invalid",
        ])
        .args(args)
        .current_dir(workspace)
        .envs(env.iter().map(|(key, value)| (key, value)))
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?}");
}

/// Everything the backend may write lives under `root`.
fn isolation(root: &Path) -> Vec<(String, OsString)> {
    let home = root.join("home");
    let tmp = root.join("tmp");
    let dir = |name: &str| root.join(name).into_os_string();
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    vec![
        ("HOME".into(), home.clone().into_os_string()),
        ("USERPROFILE".into(), home.clone().into_os_string()),
        ("TMPDIR".into(), tmp.clone().into_os_string()),
        ("TMP".into(), tmp.clone().into_os_string()),
        ("TEMP".into(), tmp.into_os_string()),
        ("GENTLE_PI_CONFIG_HOME".into(), dir("config")),
        ("GENTLE_PI_AGENT_HOME".into(), dir("agent")),
        ("PI_CODING_AGENT_DIR".into(), dir("agent")),
        ("GENTLE_SHELL_INTERACTIVE_HOST".into(), "1".into()),
        ("GENTLE_SHELL_NO_AUTO_SETUP".into(), "1".into()),
        ("GENTLE_PI_SKIP_GENTLE_AI_INSTALL".into(), "1".into()),
        ("PI_OFFLINE".into(), "1".into()),
        ("PI_NO_LOCAL_LLM".into(), "1".into()),
        ("NO_COLOR".into(), "1".into()),
        ("GIT_CONFIG_GLOBAL".into(), null.into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        (
            "XDG_CONFIG_HOME".into(),
            home.join(".config").into_os_string(),
        ),
        (
            "XDG_DATA_HOME".into(),
            home.join(".local/share").into_os_string(),
        ),
        (
            "XDG_STATE_HOME".into(),
            home.join(".local/state").into_os_string(),
        ),
        (
            "XDG_CACHE_HOME".into(),
            home.join(".cache").into_os_string(),
        ),
        ("GENTLE_SHELL_PREFIX".into(), OsString::new()),
        ("GENTLE_SHELL_BIN".into(), OsString::new()),
        ("GENTLE_SHELL_CONFIG_HOME".into(), OsString::new()),
    ]
}

/// The `gentle-ai` the launcher's checkout carries, to change the switch
/// behind Desktop's back the way the TUI's `/gentle:review-mode` would.
fn gentle_ai(launcher: &Path) -> Option<PathBuf> {
    let checkout = launcher.parent()?.parent()?;
    std::fs::read_dir(checkout.join(".gentle-ai"))
        .ok()?
        .flatten()
        .map(|entry| entry.path().join("gentle-ai"))
        .find(|path| path.is_file())
}

#[test]
#[ignore = "live: set GENTLE_SHELL_LIVE_LAUNCHER to .../bin/gentle-shell.mjs, then run with --ignored"]
fn live_review_changes_and_external_config() {
    let Some(launcher) = std::env::var_os("GENTLE_SHELL_LIVE_LAUNCHER").map(PathBuf::from) else {
        eprintln!("live: GENTLE_SHELL_LIVE_LAUNCHER is not set; skipping");
        return;
    };
    let launcher = std::fs::canonicalize(&launcher).expect("launcher");
    let node = find_node().expect("node on PATH");
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("gsg-live-review-{}-{nanos}", std::process::id()));
    for sub in ["home", "tmp", "config", "agent", "workspace"] {
        std::fs::create_dir_all(root.join(sub)).unwrap();
    }
    let env = isolation(&root);
    let workspace = root.join("workspace");
    git(&workspace, &env, &["init", "-q", "-b", "main"]);
    std::fs::write(workspace.join("notes.txt"), "one\n").unwrap();
    git(&workspace, &env, &["add", "notes.txt"]);
    git(&workspace, &env, &["commit", "-q", "-m", "init"]);
    std::fs::write(workspace.join("notes.txt"), "one\ntwo\n").unwrap();
    std::fs::write(workspace.join("fresh.txt"), "new\n").unwrap();

    let spec = LaunchSpec {
        program: node.clone(),
        args: vec![
            launcher.to_string_lossy().into_owned(),
            "--mode".into(),
            "rpc".into(),
        ],
        cwd: workspace.clone(),
        env: env.clone(),
        launcher: node,
        bundled: None,
    };
    let (process, incoming) = PiProcess::spawn(&spec).expect("spawn");
    let (tx, rx) = std::sync::mpsc::sync_channel(256);
    std::thread::spawn(move || {
        while let Ok(item) = incoming.recv_blocking() {
            if tx.send(item).is_err() {
                break;
            }
        }
    });
    let mut live = Live {
        process,
        incoming: rx,
        desktop: Desktop::new(),
        root: root.clone(),
    };

    // The probe, as the tab runs it once Pi lists the extension.
    let commands = smol::block_on(live.process.request(json!({"type": "get_commands"})))
        .expect("get_commands");
    let registered = commands.data["commands"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|command| command["name"] == "gentle:desktop");
    assert!(registered, "the Desktop extension is listed");
    let probe = live.desktop.ready(true).sends;
    live.send(probe);
    assert!(live.until("the probe", STEP, |d| d.step() == Step::Ready));
    let capabilities = live.desktop.capabilities().unwrap().clone();
    live.pump();

    if !capabilities.supports(DesktopMethod::GetChanges) {
        // 4.8.0: nothing new is sent, nothing new shows.
        live.desktop.review_refresh();
        live.desktop.changes_mark_stale();
        assert!(live.desktop.review_pump().sends.is_empty());
        assert!(live.desktop.changes_pump().sends.is_empty());
        assert!(live.desktop.review().mode().is_none());
        assert!(live.desktop.changes().list().is_none());
        eprintln!("live: an older Shell; review and changes stay hidden");
        return;
    }

    // ── Session changes ──
    assert!(live.until("the change set", STEP, |d| d.changes().list().is_some()));
    let list = live.desktop.changes().list().unwrap().clone();
    eprintln!("live: get_changes -> {list:?}");
    let notes = list
        .files
        .iter()
        .find(|file| file.path == "notes.txt")
        .expect("notes.txt listed");
    assert_eq!(notes.status, ChangeStatus::Modified);
    assert!(list.files.iter().any(|file| file.path == "fresh.txt"));
    assert!(list.roots.iter().all(|root| !root.label.contains('/')));
    let key = FileKey {
        root: notes.root.clone(),
        path: notes.path.clone(),
    };
    live.desktop.changes_select(Some(key));
    live.pump();
    assert!(live.until("the diff", STEP, |d| matches!(
        d.changes().diff(),
        Some(FileDiff::Ready(_) | FileDiff::Failed(_))
    )));
    let Some(FileDiff::Ready(diff)) = live.desktop.changes().diff() else {
        panic!("diff failed: {:?}", live.desktop.changes().diff());
    };
    assert!(diff.diff.contains("+two"), "{diff:?}");
    eprintln!(
        "live: get_change_diff notes.txt ok ({} bytes)",
        diff.diff.len()
    );

    // The agent edits again; the turn ends: the list and the open diff follow.
    std::fs::write(workspace.join("notes.txt"), "one\ntwo\nthree\n").unwrap();
    std::fs::write(workspace.join("later.txt"), "x\n").unwrap();
    live.desktop.changes_mark_stale();
    live.pump();
    assert!(live.until("the refreshed change set", STEP, |d| {
        d.changes()
            .list()
            .is_some_and(|list| list.files.iter().any(|file| file.path == "later.txt"))
            && matches!(d.changes().diff(), Some(FileDiff::Ready(diff)) if diff.diff.contains("+three"))
    }));
    eprintln!("live: the change set and the open diff refreshed");

    // ── Review mode ──
    // Hidden for good when the read found no review owner.
    live.until("the review read", Duration::from_secs(20), |d| {
        d.review().mode().is_some()
    });
    match live.desktop.review().mode().copied() {
        None => eprintln!("live: no review owner here (gentle-ai missing); the chip stays hidden"),
        Some(before) => {
            eprintln!("live: get_review_mode -> {before:?}");
            assert!(live.desktop.review_request(false, ReviewScope::Clone));
            live.pump();
            assert!(live.until("the confirmation", STEP, |d| {
                d.review().confirmation().is_some()
            }));
            let shown = live.desktop.review().confirmation().unwrap().clone();
            eprintln!(
                "live: confirmation -> {:?} / {:?}",
                shown.title, shown.message
            );
            assert!(!shown.title.is_empty());

            // The switch moves behind Desktop's back (as `/gentle:review-mode`
            // in the TUI would): the approved token is now stale, and the
            // fresh confirmation replaces the one shown.
            if let Some(cli) = gentle_ai(&launcher) {
                let status = std::process::Command::new(cli)
                    .args(["review", "mode", "disable", "--scope", "global"])
                    .arg("--cwd")
                    .arg(&workspace)
                    .envs(env.iter().map(|(key, value)| (key, value)))
                    .status()
                    .expect("gentle-ai");
                assert!(status.success());
                live.desktop.review_confirm();
                live.pump();
                assert!(live.until("the fresh confirmation", STEP, |d| {
                    d.review().confirmation().is_some() && !d.review().changing()
                }));
                eprintln!(
                    "live: stale token -> fresh confirmation {:?}",
                    live.desktop.review().confirmation().unwrap().message
                );
            }

            live.desktop.review_confirm();
            live.pump();
            assert!(live.until("the applied change", STEP, |d| {
                d.review().confirmation().is_none()
                    && !d.review().changing()
                    && d.review()
                        .mode()
                        .is_some_and(|mode| mode.scopes.clone == ReviewSetting::Disabled)
            }));
            let after = *live.desktop.review().mode().unwrap();
            assert_eq!(after.effective, ReviewEffective::Disabled);
            eprintln!("live: applied -> {after:?}");

            // Back on machine-wide, then for this clone (`gentle-ai` refuses
            // to clear a clone's off while the machine-wide one stays off).
            for scope in [ReviewScope::Global, ReviewScope::Clone] {
                live.desktop.review_request(true, scope);
                live.pump();
                assert!(live.until("a confirmation", STEP, |d| {
                    d.review().confirmation().is_some()
                }));
                eprintln!(
                    "live: turn on {scope:?} asks {:?}",
                    live.desktop.review().confirmation().unwrap().message
                );
                live.desktop.review_confirm();
                live.pump();
                assert!(live.until("applied", STEP, |d| {
                    d.review().confirmation().is_none() && !d.review().changing()
                }));
                eprintln!(
                    "live: turned on {scope:?} -> {:?}",
                    live.desktop.review().mode()
                );
            }
            let restored = *live.desktop.review().mode().unwrap();
            assert_eq!(restored.effective, ReviewEffective::Enabled, "{restored:?}");
            assert_ne!(restored.source, ReviewSource::Unknown);
            eprintln!("live: restored -> {restored:?}");
        }
    }

    // ── External config change (#3335) ──
    live.desktop.take_config_changes();
    std::fs::write(
        root.join("config").join("profiles.json"),
        "{\n  \"kind\": \"gentle-pi.agent_model_profiles\",\n  \"version\": 1,\n  \"profiles\": {}\n}\n",
    )
    .unwrap();
    let mut changes = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline && changes.is_empty() {
        live.until(
            "an external config_changed",
            Duration::from_millis(500),
            |_| false,
        );
        changes = live.desktop.take_config_changes();
    }
    eprintln!("live: external config_changed -> {changes:?}");
    assert!(
        changes
            .iter()
            .any(|change| change.store == ConfigStore::Profiles),
        "the Profiles screen is told to re-read profiles"
    );
}
