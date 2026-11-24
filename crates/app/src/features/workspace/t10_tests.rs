use super::t8_verifier::{active, category, fixture};
use gpui_kit::TestAppContext;

use super::{Category, PaneKind, PaneTree, SavedPane, SavedTab, Tab, TabState};

fn saved_terminal_tab(id: usize, cwd: std::path::PathBuf) -> Tab {
    Tab {
        id,
        pinned: false,
        state: TabState::Unbuilt(SavedTab {
            layout: PaneTree::Leaf(SavedPane {
                kind: PaneKind::Terminal,
                cwd,
                path: None,
                session: None,
            }),
            pinned: false,
            focused: 0,
        }),
    }
}

#[gpui::test]
fn t10_create_workspace_names_after_folder(cx: &mut TestAppContext) {
    let (host, cx, root, _trace) = fixture(cx, "create-name", false);
    let gamma = root.join("Gamma");
    std::fs::create_dir(&gamma).unwrap();

    cx.update(|window, cx| {
        host.update(cx, |w, cx| {
            let index = w.push_space(None, gamma.clone());
            w.spaces[index].tabs = vec![saved_terminal_tab(31, gamma.clone())];
            w.switch_workspace(index, window, cx);
        });
    });
    cx.run_until_parked();

    let created = cx.update(|_, cx| {
        host.read(cx)
            .spaces
            .iter()
            .find(|s| s.root == gamma)
            .map(|s| s.name.clone())
    });
    assert_eq!(created, Some("Gamma".into()));
    assert_eq!(cx.update(|_, cx| host.read(cx).spaces.len()), 3);
}

#[gpui::test]
fn t10_create_workspace_cancel_creates_nothing(cx: &mut TestAppContext) {
    let (host, cx, _root, _trace) = fixture(cx, "create-cancel", false);
    let palette_key = if cfg!(target_os = "macos") {
        "cmd-k"
    } else {
        "ctrl-k"
    };

    cx.simulate_keystrokes(palette_key);
    cx.simulate_input("Create workspace");
    cx.simulate_keystrokes("enter");
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|o| {
        assert!(!o.files && o.directories && !o.multiple);
        assert_eq!(o.prompt.as_deref(), Some("Create workspace"));
        None
    });
    cx.run_until_parked();

    assert_eq!(cx.update(|_, cx| host.read(cx).spaces.len()), 2);
}

#[gpui::test]
fn t10_delete_workspace_picker_deletes_after_confirmation(cx: &mut TestAppContext) {
    let (host, cx, _root, _trace) = fixture(cx, "delete-picker", false);
    let palette_key = if cfg!(target_os = "macos") {
        "cmd-k"
    } else {
        "ctrl-k"
    };

    cx.simulate_keystrokes(palette_key);
    cx.simulate_input("Delete workspace");
    cx.simulate_keystrokes("enter");
    assert_eq!(category(&host, cx), Some(Category::Workspaces));

    // The first workspace row is selected; Enter deletes it.
    cx.simulate_keystrokes("enter");
    // Confirm the alert dialog.
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    assert_eq!(cx.update(|_, cx| host.read(cx).spaces.len()), 1);
}

#[gpui::test]
fn t10_delete_current_workspace_switches_to_last_active(cx: &mut TestAppContext) {
    let (host, cx, _root, _trace) = fixture(cx, "delete-current", false);
    assert_eq!(active(&host, cx).0, 0);

    cx.update(|window, cx| {
        host.update(cx, |w, cx| w.delete_workspace(0, window, cx));
    });

    cx.update(|_, cx| {
        let w = host.read(cx);
        assert_eq!(w.spaces.len(), 1);
        assert_eq!(w.current, 0);
        assert_eq!(w.spaces[0].name, "Beta");
    });
}

#[gpui::test]
fn t10_delete_last_workspace_is_ignored(cx: &mut TestAppContext) {
    let (host, cx, _root, _trace) = fixture(cx, "delete-last", false);

    cx.update(|window, cx| {
        host.update(cx, |w, cx| w.delete_workspace(1, window, cx));
    });
    cx.update(|window, cx| {
        host.update(cx, |w, cx| w.delete_workspace(0, window, cx));
    });

    assert_eq!(cx.update(|_, cx| host.read(cx).spaces.len()), 1);
}
