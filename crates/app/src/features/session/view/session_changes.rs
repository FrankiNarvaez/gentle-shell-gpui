//! "Changed in this session": a chip over the composer with the files the
//! agent's session changed (Gentle Shell's `/gentle:changes` owner, over
//! `get_changes`), and a card listing them with their line counts and the
//! selected file's diff in the app's diff viewer (`get_change_diff`). Roots
//! are opaque labels; nothing here is a path on this machine. Files ›
//! Changes stays the repository's own Git view.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::diff::{Diff, DiffFile, DiffHoverHighlight, DiffState};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use pi_rpc::desktop_rpc::{ChangeDiff, ChangeStatus, Changes};

use crate::features::session::SessionView;
use crate::features::session::state::desktop::changes::{FileDiff, FileKey};
use crate::shared::ui;

/// What a file row asks for: show its diff, or close the one shown.
type Select = Rc<dyn Fn(Option<FileKey>, &mut Window, &mut App)>;
/// The card opened: read the change set again.
type Opened = Rc<dyn Fn(&mut Window, &mut App)>;

/// The card's diff area, rebuilt only when the slice's diff changed.
pub(in crate::features::session) struct Viewer {
    revision: u64,
    body: Body,
    truncated: bool,
    _observe: Option<Subscription>,
}

#[derive(Clone)]
enum Body {
    Loading,
    Diff(Entity<DiffState>),
    Notice(SharedString),
}

impl SessionView {
    /// Bring the card's diff viewer up to the slice's answer. Called after
    /// every changes decision and selection; the viewer is built here, not
    /// while drawing.
    pub(in crate::features::session) fn sync_changes_viewer(&mut self, cx: &mut Context<Self>) {
        let revision = self.desktop.changes().diff_revision();
        if self
            .live
            .changes_viewer
            .as_ref()
            .map(|viewer| viewer.revision)
            == Some(revision)
        {
            return;
        }
        self.live.changes_viewer = self
            .desktop
            .changes()
            .diff()
            .map(|diff| viewer(revision, diff, cx));
    }

    /// The chip, while the session has changes (or the last read failed);
    /// nothing on a Shell without `get_changes`.
    pub(super) fn render_changes_chip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let set = self.desktop.changes();
        let error = set.error().map(str::to_string);
        let list = set.list().cloned();
        if error.is_none() && list.as_ref().is_none_or(|list| list.files.is_empty()) {
            return None;
        }
        let selected = set.selected().cloned();
        let viewer = self
            .live
            .changes_viewer
            .as_ref()
            .map(|viewer| (viewer.body.clone(), viewer.truncated));
        let opened = cx.entity().downgrade();
        let chosen = cx.entity().downgrade();
        Some(
            changes_chip(
                list,
                error,
                selected,
                viewer,
                Rc::new(move |_, cx| {
                    let _ = opened.update(cx, |this, cx| this.refresh_session_changes(cx));
                }),
                Rc::new(move |file, _, cx| {
                    let _ = chosen.update(cx, |this, cx| this.select_session_change(file, cx));
                }),
            )
            .into_any_element(),
        )
    }
}

/// The diff area for one answer: the viewer, a wait, or why there is none.
fn viewer<T: 'static>(revision: u64, diff: &FileDiff, cx: &mut Context<T>) -> Viewer {
    let (body, truncated) = match diff {
        FileDiff::Loading => (Body::Loading, false),
        FileDiff::Failed(message) => (Body::Notice(message.clone().into()), false),
        FileDiff::Ready(diff) => match diff_file(diff) {
            Ok(file) => (
                Body::Diff(cx.new(|cx| DiffState::new([file], cx))),
                diff.truncated,
            ),
            Err(notice) => (Body::Notice(notice.into()), false),
        },
    };
    // Syntax highlighting arrives later: draw again then.
    let observe = match &body {
        Body::Diff(state) => Some(cx.observe(state, |_, _, cx| cx.notify())),
        _ => None,
    };
    Viewer {
        revision,
        body,
        truncated,
        _observe: observe,
    }
}

/// The one file of a `get_change_diff` answer, or what to say instead.
fn diff_file(diff: &ChangeDiff) -> Result<DiffFile, &'static str> {
    if diff.diff.trim().is_empty() {
        return Err("Git reports no difference for this file.");
    }
    let file = DiffFile::parse(&diff.diff)
        .ok()
        .and_then(|files| files.into_iter().next());
    match file {
        Some(file) if file.is_binary() => Err("Binary changes can't be shown as text."),
        Some(file) => Ok(file),
        // A cut on a line boundary can still leave a hunk short of its
        // header's counts.
        None if diff.truncated => Err("This diff is too large to show here."),
        None => Err("This diff couldn't be read."),
    }
}

/// The chip's text: files and line totals.
fn chip_label(changes: Option<&Changes>) -> String {
    let Some(changes) = changes else {
        return "Changes".to_string();
    };
    let (added, deleted) = totals(changes);
    let files = changes.files.len();
    let more = if changes.truncated { "+" } else { "" };
    format!(
        "{files}{more} {} +{added} −{deleted}",
        if files == 1 { "file" } else { "files" }
    )
}

fn totals(changes: &Changes) -> (u64, u64) {
    changes.files.iter().fold((0, 0), |(added, deleted), file| {
        (added + file.added, deleted + file.deleted)
    })
}

/// Git's one-letter status, and its color in Files › Changes.
fn status_mark(status: ChangeStatus) -> (&'static str, Hsla) {
    match status {
        ChangeStatus::Modified => ("M", ui::warning()),
        ChangeStatus::Added => ("A", ui::success()),
        ChangeStatus::Untracked => ("U", ui::success()),
        ChangeStatus::Deleted => ("D", ui::danger()),
        ChangeStatus::Renamed => ("R", ui::info()),
    }
}

/// The chip and its card. Opening it reads the change set again.
fn changes_chip(
    list: Option<Rc<Changes>>,
    error: Option<String>,
    selected: Option<FileKey>,
    viewer: Option<(Body, bool)>,
    on_open: Opened,
    on_select: Select,
) -> Popover {
    let label = chip_label(list.as_deref());
    let trigger = Button::new("session-changes-chip")
        .debug_selector(|| "session-changes-chip".into())
        .ghost()
        .xsmall()
        .text_color(if error.is_some() {
            ui::warning()
        } else {
            ui::muted()
        })
        .icon(Icon::new(IconName::Diff))
        .label(label)
        .tooltip("Changed in this session");
    Popover::new("session-changes-popover")
        .anchor(Anchor::BottomRight)
        .trigger(trigger)
        .w(rems(560. / 16.))
        .appearance(false)
        .on_open_change(move |open, window, cx| {
            if *open {
                on_open(window, cx);
            }
        })
        .content(move |_, _, cx| {
            let totals = list.as_deref().map(totals);
            let header = h_flex()
                .gap_2()
                .child(Icon::new(IconName::Diff).small().text_color(ui::muted()))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(ui::fg())
                        .child("Changed in this session"),
                )
                .child(div().flex_1())
                .when_some(totals, |this, (added, deleted)| {
                    this.child(
                        h_flex()
                            .gap_1p5()
                            .text_size(ui::code_rems(12.))
                            .font_family(ui::mono())
                            .child(div().text_color(ui::success()).child(format!("+{added}")))
                            .child(div().text_color(ui::danger()).child(format!("−{deleted}"))),
                    )
                });
            let notes = error
                .iter()
                .map(|error| (error.clone(), ui::warning()))
                .chain(list.as_ref().filter(|list| list.truncated).map(|list| {
                    (
                        format!("Only the first {} files are listed.", list.files.len()),
                        ui::faint(),
                    )
                }))
                .map(|(text, color)| {
                    div()
                        .text_size(rems(11. / 16.))
                        .text_color(color)
                        .whitespace_normal()
                        .child(text)
                });
            let rows = list
                .as_ref()
                .map(|list| file_rows(list, selected.as_ref(), &on_select))
                .unwrap_or_default();
            let diff = viewer.clone().map(|(body, truncated)| {
                let area = match body {
                    Body::Loading => h_flex()
                        .size_full()
                        .justify_center()
                        .gap_2()
                        .text_size(rems(12. / 16.))
                        .text_color(ui::muted())
                        .child(ui::spinner(gpui_kit::component::Size::Small, ui::muted()))
                        .child("Reading the diff…")
                        .into_any_element(),
                    Body::Notice(text) => h_flex()
                        .size_full()
                        .justify_center()
                        .text_size(rems(12. / 16.))
                        .text_color(ui::muted())
                        .child(text)
                        .into_any_element(),
                    Body::Diff(state) => Diff::new(&state)
                        .header_visible(false)
                        .hover_highlight(DiffHoverHighlight::Line)
                        .size_full()
                        .into_any_element(),
                };
                v_flex()
                    .gap_1()
                    .child(
                        ui::well_box()
                            .debug_selector(|| "session-changes-diff".into())
                            .h(rems(300. / 16.))
                            .overflow_hidden()
                            .child(area),
                    )
                    .when(truncated, |this| {
                        this.child(
                            div()
                                .text_size(rems(11. / 16.))
                                .text_color(ui::faint())
                                .child("Showing the first 256 KiB of this diff."),
                        )
                    })
            });
            ui::island(cx)
                .w_full()
                .child(ui::popover_in(
                    ElementId::Name("session-changes-in".into()),
                    v_flex()
                        .p_3()
                        .gap_2()
                        .child(header)
                        .children(notes)
                        .child(
                            v_flex()
                                .id("session-changes-list")
                                .max_h(rems(200. / 16.))
                                .overflow_y_scroll()
                                .gap_0p5()
                                .children(rows),
                        )
                        .children(diff),
                ))
                .into_any_element()
        })
}

/// One row per file, under its root's label when the session has several
/// roots (a subagent's worktree, say).
fn file_rows(list: &Changes, selected: Option<&FileKey>, on_select: &Select) -> Vec<AnyElement> {
    let labelled = list.roots.len() > 1;
    let mut rows = Vec::new();
    for root in &list.roots {
        if labelled {
            rows.push(
                div()
                    .pt_1()
                    .child(ui::section_label(root.label.clone()))
                    .into_any_element(),
            );
        }
        for (ix, file) in list
            .files
            .iter()
            .enumerate()
            .filter(|(_, file)| file.root == root.id)
        {
            let key = FileKey {
                root: file.root.clone(),
                path: file.path.clone(),
            };
            let is_selected = selected == Some(&key);
            let (mark, color) = status_mark(file.status);
            let on_select = on_select.clone();
            rows.push(
                h_flex()
                    .id(("session-change", ix))
                    .debug_selector(move || format!("session-change-{ix}"))
                    .w_full()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(rems(ui::ISLAND_ROW / 16.))
                    .cursor_pointer()
                    .text_size(rems(12. / 16.))
                    .hover(|s| s.bg(ui::surface_hover()))
                    .when(is_selected, |this| this.bg(ui::surface_hover()))
                    .child(
                        div()
                            .w(rems(12. / 16.))
                            .flex_none()
                            .font_family(ui::mono())
                            .text_size(ui::code_rems(12.))
                            .text_color(color)
                            .child(mark),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(ui::fg())
                            .child(file.path.clone()),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_1p5()
                            .font_family(ui::mono())
                            .text_size(ui::code_rems(12.))
                            .child(
                                div()
                                    .text_color(ui::success())
                                    .child(format!("+{}", file.added)),
                            )
                            .child(
                                div()
                                    .text_color(ui::danger())
                                    .child(format!("−{}", file.deleted)),
                            ),
                    )
                    .on_click(move |_, window, cx| on_select(Some(key.clone()), window, cx))
                    .into_any_element(),
            );
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui_kit::{
        Context, Entity, IntoElement, Modifiers, ParentElement as _, Render, Styled as _,
        TestAppContext, VisualTestContext, Window, div, px,
    };
    use pi_rpc::desktop_rpc::{
        Capabilities, ChangeDiff, DesktopSession, PROMPT_COMMAND, SCHEMA, WIDGET_KEY,
    };
    use pi_rpc::{UiMethod, UiRequest};
    use serde_json::{Value, json};

    use super::{Body, Viewer, changes_chip, chip_label, diff_file, viewer};
    use crate::features::session::state::desktop::changes::ChangeSet;

    #[test]
    fn the_chip_counts_files_and_lines() {
        let changes = serde_json::from_value(json!({
            "roots": [{"id": "root-1", "label": "app"}],
            "files": [
                {"root": "root-1", "path": "a.rs", "status": "modified", "added": 3, "deleted": 1},
                {"root": "root-1", "path": "b.rs", "status": "added", "added": 4, "deleted": 0},
            ],
            "truncated": true,
        }))
        .unwrap();
        assert_eq!(chip_label(Some(&changes)), "2+ files +7 −1");
        assert_eq!(chip_label(None), "Changes");
    }

    #[test]
    fn diffs_that_cannot_be_drawn_say_why() {
        let diff = |text: &str, truncated: bool| ChangeDiff {
            root: "root-1".into(),
            path: "a.txt".into(),
            diff: text.into(),
            truncated,
        };
        assert!(diff_file(&diff("", false)).is_err());
        let ok =
            "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1,2 @@\n one\n+two\n";
        let file = diff_file(&diff(ok, false)).unwrap();
        assert_eq!((file.additions(), file.deletions()), (1, 0));
    }

    /// Stands in for the tab: the real slice and session, the real chip
    /// and card, and the requests the slice hands out.
    struct Host {
        changes: ChangeSet,
        session: DesktopSession,
        sent: Rc<RefCell<Vec<Value>>>,
        viewer: Option<Viewer>,
    }

    impl Host {
        fn pump(&mut self) {
            for send in self.changes.pump(&mut self.session).sends {
                let payload: Value = serde_json::from_str(send.request.payload()).unwrap();
                self.sent.borrow_mut().push(payload);
            }
        }

        fn answer(&mut self, data: Value, cx: &mut Context<Self>) {
            let id = self.session.pending().unwrap().0.to_string();
            let record = json!({
                "schema": SCHEMA, "kind": "response", "requestId": id, "ok": true, "data": data,
            });
            let _ = self.changes.apply(
                &mut self.session,
                &UiRequest {
                    id: "ui-1".into(),
                    timeout: None,
                    method: UiMethod::SetWidget {
                        key: WIDGET_KEY.into(),
                        lines: Some(vec![record.to_string()]),
                        placement: None,
                    },
                },
            );
            self.sync_viewer(cx);
        }

        fn sync_viewer(&mut self, cx: &mut Context<Self>) {
            let revision = self.changes.diff_revision();
            let diff = self.changes.diff().cloned();
            self.viewer = diff.map(|diff| viewer(revision, &diff, cx));
        }
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let opened = cx.entity().downgrade();
            let chosen = cx.entity().downgrade();
            let chip = changes_chip(
                self.changes.list().cloned(),
                self.changes.error().map(str::to_string),
                self.changes.selected().cloned(),
                self.viewer
                    .as_ref()
                    .map(|viewer| (viewer.body.clone(), viewer.truncated)),
                Rc::new(move |_, cx| {
                    let _ = opened.update(cx, |host: &mut Host, cx| {
                        host.changes.mark_stale();
                        host.pump();
                        cx.notify();
                    });
                }),
                Rc::new(move |file, _, cx| {
                    let _ = chosen.update(cx, |host: &mut Host, cx| {
                        host.changes.select(file);
                        host.sync_viewer(cx);
                        host.pump();
                        cx.notify();
                    });
                }),
            );
            div()
                .size_full()
                .p_4()
                .flex()
                .flex_col()
                .justify_end()
                .child(div().w(px(700.)).flex().justify_end().child(chip))
        }
    }

    const LIST: &str = r#"{
        "roots": [{"id": "root-1", "label": "workspace"}, {"id": "root-2", "label": "helper"}],
        "files": [
            {"root": "root-1", "path": "notes.txt", "status": "modified", "added": 1, "deleted": 0},
            {"root": "root-2", "path": "fresh.txt", "status": "untracked", "added": 1, "deleted": 0}
        ],
        "truncated": false
    }"#;

    fn open(cx: &mut TestAppContext) -> (Entity<Host>, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
        });
        let (host, cx) = cx.add_window_view(|_, _| {
            let mut changes = ChangeSet::default();
            changes.set_capabilities(Capabilities {
                methods: vec!["get_changes".into(), "get_change_diff".into()],
                events: Vec::new(),
                schema: Some(SCHEMA.into()),
                command: Some(PROMPT_COMMAND.into()),
                widget: Some(WIDGET_KEY.into()),
            });
            Host {
                changes,
                session: DesktopSession::new(),
                sent: Rc::new(RefCell::new(Vec::new())),
                viewer: None,
            }
        });
        host.update(cx, |host, cx| {
            host.pump();
            host.answer(serde_json::from_str(LIST).unwrap(), cx);
            cx.notify();
        });
        cx.run_until_parked();
        (host, cx)
    }

    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        cx.run_until_parked();
        let at = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("nothing drawn as {selector}"))
            .center();
        cx.simulate_click(at, Modifiers::none());
        cx.run_until_parked();
    }

    fn last_sent(host: &Entity<Host>, cx: &mut VisualTestContext) -> Value {
        host.read_with(cx, |host, _| host.sent.borrow().last().unwrap().clone())
    }

    #[gpui::test]
    fn a_file_of_the_session_opens_its_diff_in_the_card(cx: &mut TestAppContext) {
        let (host, cx) = open(cx);
        click(cx, "session-changes-chip");
        // Opening read the change set again.
        assert_eq!(last_sent(&host, cx)["method"], "get_changes");
        host.update(cx, |host, cx| {
            host.answer(serde_json::from_str(LIST).unwrap(), cx);
            cx.notify();
        });

        click(cx, "session-change-1");
        let asked = last_sent(&host, cx);
        assert_eq!(asked["method"], "get_change_diff");
        assert_eq!(
            asked["params"],
            json!({"root": "root-2", "path": "fresh.txt"})
        );
        host.update(cx, |host, cx| {
            host.answer(
                json!({
                    "root": "root-2", "path": "fresh.txt", "truncated": false,
                    "diff": "diff --git a/fresh.txt b/fresh.txt\nnew file mode 100644\n--- /dev/null\n+++ b/fresh.txt\n@@ -0,0 +1 @@\n+hello\n",
                }),
                cx,
            );
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("session-changes-diff").is_some());
        host.read_with(cx, |host, _| {
            assert!(matches!(
                host.viewer
                    .as_ref()
                    .map(|viewer| (&viewer.body, viewer.truncated)),
                Some((Body::Diff(_), false))
            ));
        });

        // The same row again closes the diff.
        click(cx, "session-change-1");
        assert!(cx.debug_bounds("session-changes-diff").is_none());
    }
}
