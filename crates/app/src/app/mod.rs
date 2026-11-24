//! Composition root: starts GPUI, installs themes, key bindings and menus,
//! and opens the window around the workspace.

pub mod integration;

use std::path::PathBuf;
use std::rc::Rc;

use gentle_core::{interface_size, keymap};
use gpui_kit::component::{Theme, ThemeMode, TitleBar};
use gpui_kit::*;

use crate::features::workspace::{
    ActivateTab1, ActivateTab2, ActivateTab3, ActivateTab4, ActivateTab5, ActivateTab6,
    ActivateTab7, ActivateTab8, ActivateTab9, ClosePane, CloseTab, FocusChanges, FocusComposer,
    FocusFiles, FocusNextPane, FocusPrevPane, NewCanvasTab, NewTab, NewTerminalTab, NextTab,
    PrevTab, Quit, ReopenClosed, SplitDown, SplitRight, TilePane, ToggleSidebar, Workspace,
};
use crate::features::workspace::{
    ExitPaneMode, FocusPaneDown, FocusPaneLeft, FocusPaneRight, FocusPaneUp, ResizePaneDown,
    ResizePaneLeft, ResizePaneRight, ResizePaneUp, SplitLeft, SplitUp, TogglePaneMode,
};
use crate::{
    features::app_update, features::files, features::session, features::workspace,
    platform::instance, shared::assets, shared::themes,
};

actions!(gentle, [ZoomIn, ZoomOut, ActualSize]);

/// Key context of the workspace while pane mode is on.
const PANE_MODE: &str = "GentleWorkspace && pane_mode";

/// The leader (one-shot) bindings as `(keystrokes, action)`, all built from
/// `prefix` (the user's leader, `keymap.json`; `ctrl-space` by default).
fn leader_bindings(prefix: &str) -> Vec<(String, Box<dyn Action>)> {
    fn entry<A: Action>(prefix: &str, key: &str, action: A) -> (String, Box<dyn Action>) {
        (format!("{prefix} {key}"), Box::new(action))
    }
    let p = prefix;
    vec![
        entry(p, "h", FocusPaneLeft),
        entry(p, "j", FocusPaneDown),
        entry(p, "k", FocusPaneUp),
        entry(p, "l", FocusPaneRight),
        entry(p, "v", SplitRight),
        entry(p, "s", SplitDown),
        entry(p, "x", ClosePane),
        entry(p, "u", ReopenClosed),
        entry(p, "c", NewTab),
        entry(p, "t", NewTerminalTab),
        entry(p, "n", NextTab),
        entry(p, "p", PrevTab),
        entry(p, "f", FocusFiles),
        entry(p, "g", FocusChanges),
        entry(p, "i", FocusComposer),
        entry(p, "b", ToggleSidebar),
        // `:` is shift-; on most layouts; platforms report it either way.
        entry(p, ":", workspace::OpenPalette),
        entry(p, "shift-;", workspace::OpenPalette),
        entry(p, "space", TogglePaneMode),
        entry(p, "1", ActivateTab1),
        entry(p, "2", ActivateTab2),
        entry(p, "3", ActivateTab3),
        entry(p, "4", ActivateTab4),
        entry(p, "5", ActivateTab5),
        entry(p, "6", ActivateTab6),
        entry(p, "7", ActivateTab7),
        entry(p, "8", ActivateTab8),
        entry(p, "9", ActivateTab9),
        // `?` is shift-/ on most layouts.
        entry(p, "?", workspace::OpenShortcuts),
        entry(p, "shift-/", workspace::OpenShortcuts),
    ]
}

/// Sticky pane mode bindings: active while the workspace root carries the
/// `pane_mode` context flag.
fn pane_mode_bindings() -> Vec<KeyBinding> {
    let c = Some(PANE_MODE);
    vec![
        KeyBinding::new("h", FocusPaneLeft, c),
        KeyBinding::new("j", FocusPaneDown, c),
        KeyBinding::new("k", FocusPaneUp, c),
        KeyBinding::new("l", FocusPaneRight, c),
        KeyBinding::new("shift-h", ResizePaneLeft, c),
        KeyBinding::new("shift-j", ResizePaneDown, c),
        KeyBinding::new("shift-k", ResizePaneUp, c),
        KeyBinding::new("shift-l", ResizePaneRight, c),
        KeyBinding::new("v", SplitRight, c),
        KeyBinding::new("s", SplitDown, c),
        KeyBinding::new("x", ClosePane, c),
        KeyBinding::new("escape", ExitPaneMode, c),
        KeyBinding::new("enter", ExitPaneMode, c),
    ]
}

/// Runs the app until its last window closes. `requests` carries later
/// launches' folders and prompts to this window.
pub fn run(
    folder: Option<PathBuf>,
    prompt: Option<String>,
    url: Option<String>,
    requests: async_channel::Receiver<instance::Incoming>,
) {
    gpui_kit::application()
        .with_assets(assets::AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            Theme::change(ThemeMode::Dark, None, cx);
            themes::init(cx);
            crate::features::browser::init(cx);
            #[cfg(target_os = "macos")]
            crate::platform::icons::set_dock_icon();
            app_update::init(cx);
            crate::features::hydration::init(cx);
            crate::shared::kisses::init(cx);
            crate::features::genui::mode::init(cx);
            crate::features::session::input_style::init(cx);
            crate::shared::vim_keys::init(cx);
            crate::features::backend::init(cx);
            crate::features::diagnostics::start(cx);

            let key_messages = bind_keys(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.on_action(|_: &ZoomIn, cx| step_interface_size(interface_size::bigger, cx));
            cx.on_action(|_: &ZoomOut, cx| step_interface_size(interface_size::smaller, cx));
            cx.on_action(|_: &ActualSize, cx| {
                themes::set_interface_size(interface_size::DEFAULT, cx)
            });
            set_menus(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1280.), px(820.)), cx)),
                window_min_size: Some(size(px(720.), px(480.))),
                window_background: WindowBackgroundAppearance::Blurred,
                titlebar: Some(TitlebarOptions {
                    title: Some("Gentle Shell".into()),
                    appears_transparent: true,
                    // Centered in the 36 px title bar (workspace.rs).
                    traffic_light_position: Some(point(px(14.), px(11.))),
                }),
                app_id: Some("dev.gentle-shell.gpui".into()),
                #[cfg(target_os = "linux")]
                icon: crate::platform::icons::window_icon(),
                ..TitleBar::window_options()
            };
            integration::register(cx);
            let (handle, workspace) = gpui_kit::open_window(options, cx, |window, cx| {
                let workspace = cx.new(|cx| {
                    Workspace::new(folder.clone(), prompt.clone(), url.clone(), window, cx)
                });
                integration::attach_sidebar(&workspace, window, cx);
                workspace.update(cx, |w, cx| w.report_keymap(&key_messages, cx));
                workspace
            })
            .expect("failed to open the Gentle Shell window");
            // Requests from later launches: bring this window forward and act on them.
            // Weak, so quitting releases the workspace (and stops its tabs' backends).
            let workspace = workspace.downgrade();
            cx.spawn(async move |cx| {
                while let Ok(incoming) = requests.recv().await {
                    let request = match incoming {
                        instance::Incoming::Open(request) => request,
                        // An agent driving a page doesn't pull the window forward.
                        instance::Incoming::Browser { op, reply } => {
                            let answered = handle.update(cx, |_, window, cx| {
                                workspace.update(cx, |w, cx| {
                                    w.browser_request(op, reply.clone(), window, cx)
                                })
                            });
                            if !matches!(answered, Ok(Ok(()))) {
                                let _ = reply.send(gentle_core::browser::error(
                                    "closed",
                                    "the window is closed",
                                ));
                            }
                            continue;
                        }
                    };
                    // An agent's `gentle-device` doesn't pull the window forward.
                    let activate = request.device.is_none();
                    let _ = handle.update(cx, |_, window, cx| {
                        if activate {
                            window.activate_window();
                        }
                        let _ = workspace.update(cx, |w, cx| w.open_request(request, window, cx));
                    });
                    if activate {
                        cx.update(|cx| cx.activate(true));
                    }
                }
            })
            .detach();
            cx.activate(true);
        });
}

/// The interface size one step from the current one, in every window.
fn step_interface_size(step: fn(u16) -> u16, cx: &mut App) {
    let now = cx.global::<themes::Themes>().interface_size;
    themes::set_interface_size(step(now), cx);
}

/// Bindings other code registered before the app's own (text inputs, …): a
/// reload clears the keymap and puts these back.
struct Foreign(Vec<KeyBinding>);

impl Global for Foreign {}

/// The leader the keymap was last bound with.
struct ActiveLeader(String);

impl Global for ActiveLeader {}

/// The leader key in use: the user's `keymap.json` one, else the default.
pub fn leader(cx: &App) -> String {
    cx.try_global::<ActiveLeader>()
        .map(|l| l.0.clone())
        .unwrap_or_else(|| gentle_core::keymap::DEFAULT_LEADER.to_string())
}

/// Binds every app shortcut: the defaults, then the user's `keymap.json` on
/// top (later bindings win). Calling it again reloads the keymap. Returns what
/// was wrong in the file, one line each; bad entries are skipped.
pub fn bind_keys(cx: &mut App) -> Vec<String> {
    if cx.has_global::<Foreign>() {
        let foreign = cx.global::<Foreign>().0.clone();
        cx.clear_key_bindings();
        cx.bind_keys(foreign);
    } else {
        let snapshot = cx.key_bindings().borrow().bindings().cloned().collect();
        cx.set_global(Foreign(snapshot));
    }
    let user = crate::shared::keymap::load();
    let mut messages: Vec<String> = user.errors.iter().map(|e| e.describe()).collect();
    bind_defaults(cx);
    let mut leader = Vec::new();
    for (keys, action) in leader_bindings(&user.leader) {
        match KeyBinding::load(&keys, action, None, false, None, cx) {
            Ok(binding) => leader.push(binding),
            Err(e) => messages.push(format!("keymap.json: leader {}: {e}", user.leader)),
        }
    }
    cx.bind_keys(leader);
    cx.set_global(ActiveLeader(user.leader.clone()));
    cx.bind_keys(pane_mode_bindings());
    files::bind_keys(cx);
    crate::features::canvas::bind_keys(cx);
    let mut bindings = Vec::new();
    for entry in &user.bindings {
        match user_binding(entry, cx) {
            Ok(binding) => bindings.push(binding),
            Err(e) => messages.push(
                keymap::Issue {
                    index: Some(entry.index),
                    message: e,
                }
                .describe(),
            ),
        }
    }
    cx.bind_keys(bindings);
    messages
}

/// A `keymap.json` entry as a GPUI binding; the error names what GPUI rejects
/// (an unregistered action, a context it can't parse, a keystroke).
fn user_binding(entry: &keymap::Binding, cx: &App) -> Result<KeyBinding, String> {
    let action = match &entry.action {
        None => Box::new(NoAction) as Box<dyn Action>,
        Some(name) => cx.build_action(name, None).map_err(|_| {
            format!("unknown action {name:?} (see \"Keymap file\" in docs/integrations.md)")
        })?,
    };
    let context = match &entry.context {
        None => None,
        Some(c) => Some(Rc::new(
            KeyBindingContextPredicate::parse(c).map_err(|e| format!("context {c:?}: {e}"))?,
        )),
    };
    KeyBinding::load(&entry.keys, action, context, false, None, cx)
        .map_err(|e| format!("keys {:?}: {e}", entry.keys))
}

fn bind_defaults(cx: &mut App) {
    // Cmd on macOS, Ctrl elsewhere.
    macro_rules! key {
        ($k:literal) => {
            if cfg!(target_os = "macos") {
                concat!("cmd-", $k)
            } else {
                concat!("ctrl-", $k)
            }
        };
    }
    // A new tiled pane is ctrl-shift-d off macOS: ctrl-d belongs to terminals.
    // ⌘⇧T is a new terminal tab, so reopening takes ⌥⌘T; off macOS
    // ctrl-alt-t opens the desktop's terminal, hence the extra shift.
    let (tile, split_down, workspaces, reopen) = if cfg!(target_os = "macos") {
        ("cmd-d", "cmd-shift-d", "ctrl-cmd-k", "cmd-alt-t")
    } else {
        (
            "ctrl-shift-d",
            "ctrl-shift-e",
            "ctrl-alt-k",
            "ctrl-alt-shift-t",
        )
    };
    cx.bind_keys([
        KeyBinding::new(key!("t"), NewTab, None),
        KeyBinding::new(key!("shift-t"), NewTerminalTab, None),
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-shift-e"
            } else {
                "ctrl-alt-shift-e"
            },
            NewCanvasTab,
            None,
        ),
        KeyBinding::new(key!("w"), ClosePane, None),
        KeyBinding::new(key!("shift-w"), CloseTab, None),
        KeyBinding::new(reopen, ReopenClosed, None),
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PrevTab, None),
        KeyBinding::new(tile, TilePane, None),
        KeyBinding::new(split_down, SplitDown, None),
        KeyBinding::new(key!("]"), FocusNextPane, None),
        KeyBinding::new(key!("["), FocusPrevPane, None),
        KeyBinding::new(key!("b"), ToggleSidebar, None),
        // Go to tab N (9 = last tab).
        KeyBinding::new(key!("1"), ActivateTab1, None),
        KeyBinding::new(key!("2"), ActivateTab2, None),
        KeyBinding::new(key!("3"), ActivateTab3, None),
        KeyBinding::new(key!("4"), ActivateTab4, None),
        KeyBinding::new(key!("5"), ActivateTab5, None),
        KeyBinding::new(key!("6"), ActivateTab6, None),
        KeyBinding::new(key!("7"), ActivateTab7, None),
        KeyBinding::new(key!("8"), ActivateTab8, None),
        KeyBinding::new(key!("9"), ActivateTab9, None),
        // Focus chords. Off macOS never bind plain ctrl-letter globally.
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-l"
            } else {
                "ctrl-shift-l"
            },
            FocusComposer,
            None,
        ),
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-shift-b"
            } else {
                "ctrl-shift-b"
            },
            FocusFiles,
            None,
        ),
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-shift-g"
            } else {
                "ctrl-shift-g"
            },
            FocusChanges,
            None,
        ),
        KeyBinding::new(key!("q"), Quit, None),
        KeyBinding::new("escape", workspace::CloseOverlay, Some("GentleWorkspace")),
        // Pane mode owns Esc (it exits the mode), not the agent's abort.
        KeyBinding::new(
            "escape",
            session::Abort,
            Some("GentleSession && !pane_mode"),
        ),
        KeyBinding::new(key!("."), session::Stop, Some(session::CONTEXT)),
        KeyBinding::new(
            key!("shift-a"),
            session::ToggleAgents,
            Some(session::CONTEXT),
        ),
        KeyBinding::new(key!("n"), session::NewSession, Some(session::CONTEXT)),
        KeyBinding::new(key!("k"), workspace::OpenPalette, None),
        KeyBinding::new(workspaces, workspace::OpenWorkspaces, None),
        KeyBinding::new(key!("shift-o"), workspace::OpenSessions, None),
        KeyBinding::new(key!(","), workspace::OpenSettings, None),
        // ⌘/ on macOS. Off macOS ctrl-/ is the terminals' undo (0x1f), so the
        // chord takes shift like the other global ones; `?` is shift-/.
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-/"
            } else {
                "ctrl-shift-/"
            },
            workspace::OpenShortcuts,
            None,
        ),
        // Some platforms report that chord as `?` with shift. Not on macOS:
        // ⌘? is the Help menu's search.
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-shift-?", workspace::OpenShortcuts, None),
        // Interface size. Bindings run before a terminal sees the keys. ⌘+
        // is shift-= on most layouts; the platforms report it either way.
        KeyBinding::new(key!("="), ZoomIn, None),
        KeyBinding::new(key!("+"), ZoomIn, None),
        KeyBinding::new(key!("shift-="), ZoomIn, None),
        KeyBinding::new(key!("-"), ZoomOut, None),
        KeyBinding::new(key!("0"), ActualSize, None),
    ]);
}

fn set_menus(cx: &mut App) {
    cx.set_menus(vec![Menu {
        name: "Gentle Shell".into(),
        items: vec![
            MenuItem::action("New Tab", NewTab),
            MenuItem::action("New Terminal Tab", NewTerminalTab),
            MenuItem::action("New Canvas Tab", NewCanvasTab),
            MenuItem::action("New Pane (Tiled)", TilePane),
            MenuItem::action("Split Right", SplitRight),
            MenuItem::action("Split Down", SplitDown),
            MenuItem::action("Split Left", SplitLeft),
            MenuItem::action("Split Up", SplitUp),
            MenuItem::action("Close Pane", ClosePane),
            MenuItem::action("Close Tab", CloseTab),
            MenuItem::action("Reopen Closed Pane", ReopenClosed),
            MenuItem::separator(),
            MenuItem::action("Toggle Sidebar", ToggleSidebar),
            MenuItem::action("Focus Composer", FocusComposer),
            MenuItem::action("Focus Files", FocusFiles),
            MenuItem::action("Focus Git Changes", FocusChanges),
            MenuItem::separator(),
            MenuItem::action("Configuration…", workspace::OpenSettings),
            MenuItem::action("Keyboard Shortcuts", workspace::OpenShortcuts),
            MenuItem::separator(),
            MenuItem::action("Zoom In", ZoomIn),
            MenuItem::action("Zoom Out", ZoomOut),
            MenuItem::action("Actual Size", ActualSize),
            MenuItem::separator(),
            MenuItem::action("Quit Gentle Shell", Quit),
        ],
        disabled: false,
    }]);
}

#[cfg(test)]
mod tests {
    use super::{leader_bindings, pane_mode_bindings};
    use gentle_core::keymap::DEFAULT_LEADER;

    #[test]
    fn default_leader_bindings_are_unchanged() {
        let keys: Vec<String> = leader_bindings(DEFAULT_LEADER)
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        let mut expected: Vec<String> = [
            "h", "j", "k", "l", "v", "s", "x", "u", "c", "t", "n", "p", "f", "g", "i", "b", ":",
            "shift-;", "space",
        ]
        .iter()
        .map(|k| format!("ctrl-space {k}"))
        .collect();
        expected.extend((1..=9).map(|n| format!("ctrl-space {n}")));
        expected.extend(["ctrl-space ?".to_string(), "ctrl-space shift-/".to_string()]);
        assert_eq!(keys, expected);
    }

    #[test]
    fn a_configured_leader_prefixes_every_binding() {
        for (keys, _) in leader_bindings("ctrl-a") {
            assert!(keys.starts_with("ctrl-a "), "{keys}");
            assert_eq!(keys.split(' ').count(), 2);
        }
    }

    #[test]
    fn leader_opens_the_shortcuts_screen_on_both_reports_of_question_mark() {
        use gpui_kit::Action as _;
        let keys: Vec<String> = leader_bindings(DEFAULT_LEADER)
            .iter()
            .filter(|(_, action)| action.name() == crate::features::workspace::OpenShortcuts.name())
            .map(|(keys, _)| keys.clone())
            .collect();
        assert_eq!(keys, ["ctrl-space ?", "ctrl-space shift-/"]);
    }

    #[test]
    fn action_names_match_the_cheat_sheet_labels() {
        use gpui_kit::Action as _;

        use crate::features::{session, workspace};
        use gentle_core::shortcuts::label;
        assert_eq!(
            label(workspace::OpenShortcuts.name()),
            Some("Keyboard shortcuts")
        );
        assert_eq!(
            label(workspace::FocusPaneLeft.name()),
            Some("Focus pane left")
        );
        assert_eq!(
            label(workspace::ExitPaneMode.name()),
            Some("Exit pane mode")
        );
        assert_eq!(label(session::Abort.name()), Some("Interrupt the agent"));
        assert_eq!(label(super::ZoomIn.name()), Some("Zoom in"));
    }

    #[test]
    fn pane_mode_bindings_parse() {
        assert_eq!(pane_mode_bindings().len(), 13);
    }
}
