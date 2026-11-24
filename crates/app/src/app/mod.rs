//! Composition root: starts GPUI, installs themes, key bindings and menus,
//! and opens the window around the workspace.

pub mod integration;

use std::path::PathBuf;

use gentle_core::interface_size;
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

/// The leader prefix: every one-shot leader binding is this followed by one
/// key. One value so a configurable prefix only has to change it.
pub const LEADER: &str = "ctrl-space";

/// Key context of the workspace while pane mode is on.
const PANE_MODE: &str = "GentleWorkspace && pane_mode";

/// The leader (one-shot) bindings, all built from `prefix`.
fn leader_bindings(prefix: &str) -> Vec<KeyBinding> {
    // `KeyBinding::new` wants `'static` text; these few strings live for the
    // whole run anyway and are built once at startup.
    let seq = |k: &str| -> &'static str { Box::leak(format!("{prefix} {k}").into_boxed_str()) };
    vec![
        KeyBinding::new(seq("h"), FocusPaneLeft, None),
        KeyBinding::new(seq("j"), FocusPaneDown, None),
        KeyBinding::new(seq("k"), FocusPaneUp, None),
        KeyBinding::new(seq("l"), FocusPaneRight, None),
        KeyBinding::new(seq("v"), SplitRight, None),
        KeyBinding::new(seq("s"), SplitDown, None),
        KeyBinding::new(seq("x"), ClosePane, None),
        KeyBinding::new(seq("u"), ReopenClosed, None),
        KeyBinding::new(seq("c"), NewTab, None),
        KeyBinding::new(seq("t"), NewTerminalTab, None),
        KeyBinding::new(seq("n"), NextTab, None),
        KeyBinding::new(seq("p"), PrevTab, None),
        KeyBinding::new(seq("f"), FocusFiles, None),
        KeyBinding::new(seq("g"), FocusChanges, None),
        KeyBinding::new(seq("i"), FocusComposer, None),
        KeyBinding::new(seq("b"), ToggleSidebar, None),
        // `:` is shift-; on most layouts; platforms report it either way.
        KeyBinding::new(seq(":"), workspace::OpenPalette, None),
        KeyBinding::new(seq("shift-;"), workspace::OpenPalette, None),
        KeyBinding::new(seq("space"), TogglePaneMode, None),
        KeyBinding::new(seq("1"), ActivateTab1, None),
        KeyBinding::new(seq("2"), ActivateTab2, None),
        KeyBinding::new(seq("3"), ActivateTab3, None),
        KeyBinding::new(seq("4"), ActivateTab4, None),
        KeyBinding::new(seq("5"), ActivateTab5, None),
        KeyBinding::new(seq("6"), ActivateTab6, None),
        KeyBinding::new(seq("7"), ActivateTab7, None),
        KeyBinding::new(seq("8"), ActivateTab8, None),
        KeyBinding::new(seq("9"), ActivateTab9, None),
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

            bind_keys(cx);
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

fn bind_keys(cx: &mut App) {
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
        // Interface size. Bindings run before a terminal sees the keys. ⌘+
        // is shift-= on most layouts; the platforms report it either way.
        KeyBinding::new(key!("="), ZoomIn, None),
        KeyBinding::new(key!("+"), ZoomIn, None),
        KeyBinding::new(key!("shift-="), ZoomIn, None),
        KeyBinding::new(key!("-"), ZoomOut, None),
        KeyBinding::new(key!("0"), ActualSize, None),
    ]);
    cx.bind_keys(leader_bindings(LEADER));
    cx.bind_keys(pane_mode_bindings());
    files::FileView::bind_keys(cx);
    crate::features::canvas::bind_keys(cx);
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
    use super::{LEADER, leader_bindings, pane_mode_bindings};

    #[test]
    fn leader_bindings_share_one_prefix_and_parse() {
        let bindings = leader_bindings(LEADER);
        for b in &bindings {
            let ks = b.keystrokes();
            assert_eq!(ks.len(), 2, "leader bindings are prefix + one key");
            assert!(ks[0].modifiers().control);
        }
        assert!(
            bindings
                .iter()
                .any(|b| b.keystrokes().get(1).is_some_and(|k| k.key() == ":"))
        );
        // Changing the prefix changes every binding.
        for b in leader_bindings("ctrl-a") {
            assert_eq!(b.keystrokes()[0].key(), "a");
        }
    }

    #[test]
    fn pane_mode_bindings_parse() {
        assert_eq!(pane_mode_bindings().len(), 13);
    }
}
