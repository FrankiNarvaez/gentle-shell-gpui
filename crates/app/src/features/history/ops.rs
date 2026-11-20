//! The Git pane's actions: running one off the UI thread, the inline prompt
//! for a name or a message, and the confirmations before what can't be
//! undone.

use std::path::Path;

use gentle_core::git::ops::{InProgress, PullMode, ResetMode, explain_prompt, replace_subject};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use super::{HistoryView, Outcome};
use crate::features::git;
use crate::shared::pane::{AskAgent, OpenCommitDiff, OpenTerminalAt};

/// What the inline prompt asks for.
#[derive(Clone)]
pub(super) enum Ask {
    /// A new branch at `at` (a hash, or `HEAD`), switched to when `checkout`.
    Branch { at: String, label: String },
    /// A new tag at `at`, annotated when the second field has a message.
    Tag { at: String, label: String },
    /// A stash of every change, with an optional message.
    Stash,
    /// A new remote: name and URL.
    Remote,
    /// HEAD's new message.
    Reword,
}

impl Ask {
    pub(super) fn title(&self) -> String {
        match self {
            Ask::Branch { label, .. } => format!("New branch at {label}"),
            Ask::Tag { label, .. } => format!("New tag at {label}"),
            Ask::Stash => "Stash changes".into(),
            Ask::Remote => "Add remote".into(),
            Ask::Reword => "Edit HEAD's message".into(),
        }
    }

    pub(super) fn confirm(&self) -> &'static str {
        match self {
            Ask::Branch { .. } | Ask::Tag { .. } => "Create",
            Ask::Stash => "Stash",
            Ask::Remote => "Add",
            Ask::Reword => "Save",
        }
    }
}

/// The inline prompt under the toolbar: one or two fields.
pub(super) struct Prompt {
    pub ask: Ask,
    pub first: Entity<InputState>,
    pub second: Option<Entity<InputState>>,
    _subscriptions: Vec<Subscription>,
}

impl HistoryView {
    /// Run `job` in the repository off the UI thread, saying `doing` in the
    /// footer meanwhile and `done` after, then read the history again.
    pub(super) fn run_op(
        &mut self,
        doing: &'static str,
        done: impl Into<String>,
        job: impl FnOnce(&Path) -> Result<(), String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        let top = self.toplevel.clone();
        let done = done.into();
        self.busy = Some(doing);
        self.outcome = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { job(&top) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = None;
                this.outcome = Some(match result {
                    Ok(()) => Outcome::Done(done),
                    Err(error) => Outcome::Failed(error),
                });
                this.reload(cx);
            });
        })
        .detach();
    }

    pub(super) fn fetch(&mut self, cx: &mut Context<Self>) {
        self.run_op(
            "Fetching all remotes…",
            "Fetched all remotes",
            git::ops::fetch_all,
            cx,
        );
    }

    pub(super) fn pull(&mut self, cx: &mut Context<Self>) {
        let mode = self.pull_mode;
        self.run_op(
            "Pulling…",
            "Pulled",
            move |top| git::ops::pull(top, mode),
            cx,
        );
    }

    pub(super) fn set_pull_mode(&mut self, mode: PullMode, cx: &mut Context<Self>) {
        self.pull_mode = mode;
        cx.notify();
    }

    pub(super) fn push(&mut self, cx: &mut Context<Self>) {
        self.run_op("Pushing…", "Pushed", git::push, cx);
    }

    pub(super) fn switch(&mut self, branch: String, cx: &mut Context<Self>) {
        let done = format!("Switched to {branch}");
        self.run_op(
            "Switching…",
            done,
            move |top| git::ops::switch(top, &branch),
            cx,
        );
    }

    pub(super) fn checkout_detached(&mut self, hash: String, cx: &mut Context<Self>) {
        let done = format!("Checked out {}", short(&hash));
        self.run_op(
            "Checking out…",
            done,
            move |top| git::ops::checkout_detached(top, &hash),
            cx,
        );
    }

    pub(super) fn cherry_pick(&mut self, hash: String, merge: bool, cx: &mut Context<Self>) {
        let done = format!("Cherry-picked {}", short(&hash));
        self.run_op(
            "Cherry-picking…",
            done,
            move |top| git::ops::cherry_pick(top, &hash, merge),
            cx,
        );
    }

    pub(super) fn revert(&mut self, hash: String, merge: bool, cx: &mut Context<Self>) {
        let done = format!("Reverted {}", short(&hash));
        self.run_op(
            "Reverting…",
            done,
            move |top| git::ops::revert(top, &hash, merge),
            cx,
        );
    }

    pub(super) fn reset(&mut self, hash: String, mode: ResetMode, cx: &mut Context<Self>) {
        let done = format!("Reset to {}", short(&hash));
        self.run_op(
            "Resetting…",
            done,
            move |top| git::ops::reset(top, &hash, mode),
            cx,
        );
    }

    /// Reset that drops uncommitted work asks first.
    pub(super) fn confirm_reset_discard(
        &mut self,
        hash: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let branch = self
            .read()
            .and_then(|h| h.current.clone())
            .unwrap_or_else(|| "HEAD".into());
        let this = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (this, hash) = (this.clone(), hash.clone());
            alert
                .title(format!("Reset {branch} to {}?", short(&hash)))
                .description(
                    "The commits after it leave the branch, and every uncommitted change is \
                     discarded. Changes that were never committed can't be recovered.",
                )
                .ok_text("Reset and discard")
                .ok_variant(ButtonVariant::Danger)
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    let hash = hash.clone();
                    let _ = this.update(cx, |this, cx| {
                        this.reset(hash, ResetMode::DiscardChanges, cx)
                    });
                    true
                })
        });
    }

    pub(super) fn delete_tag(&mut self, tag: String, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (this, tag) = (this.clone(), tag.clone());
            alert
                .title(format!("Delete tag {tag}?"))
                .description("The tag is removed here; copies already pushed to a remote stay.")
                .ok_text("Delete")
                .ok_variant(ButtonVariant::Danger)
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    let tag = tag.clone();
                    let _ = this.update(cx, |this, cx| {
                        let done = format!("Deleted tag {tag}");
                        this.run_op(
                            "Deleting tag…",
                            done,
                            move |top| git::ops::delete_tag(top, &tag),
                            cx,
                        )
                    });
                    true
                })
        });
    }

    pub(super) fn push_tag(&mut self, tag: String, cx: &mut Context<Self>) {
        let Some(remote) = self.default_remote() else {
            self.outcome = Some(Outcome::Failed("This repository has no remote".into()));
            cx.notify();
            return;
        };
        let done = format!("Pushed tag {tag} to {remote}");
        self.run_op(
            "Pushing tag…",
            done,
            move |top| git::ops::push_tag(top, &remote, &tag),
            cx,
        );
    }

    pub(super) fn lfs(&mut self, op: git::ops::Lfs, cx: &mut Context<Self>) {
        let remote = self.default_remote();
        self.run_op(
            "Running Git LFS…",
            format!("{} done", op.label()),
            move |top| git::ops::lfs(top, op, remote.as_deref()),
            cx,
        );
    }

    /// `origin` when there is one, else the first remote.
    pub(super) fn default_remote(&self) -> Option<String> {
        let remotes = &self.read()?.remotes;
        remotes
            .iter()
            .find(|r| *r == "origin")
            .or(remotes.first())
            .cloned()
    }

    pub(super) fn finish(&mut self, op: InProgress, abort: bool, cx: &mut Context<Self>) {
        if abort {
            self.run_op(
                "Aborting…",
                format!("{} aborted", op.label()),
                move |top| git::ops::abort_in_progress(top, op),
                cx,
            );
        } else {
            self.run_op(
                "Continuing…",
                format!("{} finished", op.label()),
                move |top| git::ops::continue_in_progress(top, op),
                cx,
            );
        }
    }

    pub(super) fn confirm_abort(
        &mut self,
        op: InProgress,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let this = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let this = this.clone();
            alert
                .title(format!("Abort the {}?", op.label().to_lowercase()))
                .description(
                    "Everything goes back to how it was before it started, including the \
                     conflicts you resolved.",
                )
                .ok_text("Abort")
                .ok_variant(ButtonVariant::Danger)
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    let _ = this.update(cx, |this, cx| this.finish(op, true, cx));
                    true
                })
        });
    }

    pub(super) fn copy_hash(&self, hash: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(hash.to_string()));
    }

    /// The full message, read off the UI thread, then copied.
    pub(super) fn copy_message(&self, hash: String, cx: &mut Context<Self>) {
        let top = self.toplevel.clone();
        cx.spawn(async move |_, cx| {
            let message = cx
                .background_executor()
                .spawn(async move { git::ops::message(&top, &hash) })
                .await;
            if let Some(message) = message {
                cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(message)));
            }
        })
        .detach();
    }

    pub(super) fn explain(&self, short: &str, subject: &str, window: &mut Window, cx: &mut App) {
        window.dispatch_action(
            Box::new(AskAgent {
                cwd: self.toplevel.clone(),
                prompt: explain_prompt(short, subject),
            }),
            cx,
        );
    }

    pub(super) fn open_terminal(&self, window: &mut Window, cx: &mut App) {
        window.dispatch_action(
            Box::new(OpenTerminalAt {
                cwd: self.toplevel.clone(),
            }),
            cx,
        );
    }

    pub(super) fn open_file_diff(&self, path: String, window: &mut Window, cx: &mut App) {
        let Some(commit) = self.selected_commit() else {
            return;
        };
        window.dispatch_action(
            Box::new(OpenCommitDiff {
                root: self.toplevel.clone(),
                path: path.into(),
                commit: commit.hash.clone(),
                parent: commit.parents.first().cloned(),
            }),
            cx,
        );
    }

    // ── The inline prompt ───────────────────────────────────────────────────

    /// Ask for a name or a message under the toolbar; `initial` fills the
    /// first field.
    pub(super) fn ask(
        &mut self,
        ask: Ask,
        initial: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (first_hint, second_hint) = match &ask {
            Ask::Branch { .. } => ("Branch name", None),
            Ask::Tag { .. } => (
                "Tag name",
                Some("Message (optional: makes an annotated tag)"),
            ),
            Ask::Stash => ("Message (optional)", None),
            Ask::Remote => ("Name", Some("URL")),
            Ask::Reword => ("Summary", None),
        };
        let first = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(first_hint)
                .default_value(initial)
        });
        let second =
            second_hint.map(|hint| cx.new(|cx| InputState::new(window, cx).placeholder(hint)));
        let mut subscriptions = Vec::new();
        for input in std::iter::once(&first).chain(second.as_ref()) {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        this.submit_prompt(window, cx);
                    }
                },
            ));
        }
        first.update(cx, |state, cx| state.focus(window, cx));
        self.prompt = Some(Prompt {
            ask,
            first,
            second,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    pub(super) fn cancel_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn submit_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prompt) = self.prompt.take() else {
            return;
        };
        let first = prompt.first.read(cx).value().to_string();
        let second = prompt
            .second
            .as_ref()
            .map(|s| s.read(cx).value().to_string())
            .unwrap_or_default();
        window.focus(&self.focus, cx);
        match prompt.ask {
            Ask::Branch { at, .. } => {
                let done = format!("Created and switched to {}", first.trim());
                self.run_op(
                    "Creating branch…",
                    done,
                    move |top| git::ops::create_branch(top, &first, &at, true),
                    cx,
                )
            }
            Ask::Tag { at, .. } => {
                let done = format!("Created tag {}", first.trim());
                self.run_op(
                    "Creating tag…",
                    done,
                    move |top| git::ops::create_tag(top, &first, &at, &second),
                    cx,
                )
            }
            Ask::Stash => self.run_op(
                "Stashing…",
                "Stashed changes",
                move |top| git::stash_push(top, &first),
                cx,
            ),
            Ask::Remote => {
                let done = format!("Added remote {}", first.trim());
                self.run_op(
                    "Adding remote…",
                    done,
                    move |top| git::ops::add_remote(top, &first, &second),
                    cx,
                )
            }
            Ask::Reword => self.run_op(
                "Saving message…",
                "Message saved",
                move |top| {
                    let old = git::ops::message(top, "HEAD").unwrap_or_default();
                    git::ops::reword_head(top, &replace_subject(&old, &first))
                },
                cx,
            ),
        }
    }

    /// Reword HEAD's summary; its body stays as it is.
    pub(super) fn reword(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(subject) = self
            .commits
            .iter()
            .find(|c| c.head)
            .map(|c| c.subject.clone())
        else {
            return;
        };
        self.ask(Ask::Reword, subject, window, cx);
    }
}

fn short(hash: &str) -> &str {
    hash.get(..7).unwrap_or(hash)
}
