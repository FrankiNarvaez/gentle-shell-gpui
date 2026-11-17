//! What this session changed, over Gentle Desktop's `get_changes` and
//! `get_change_diff`.
//!
//! The list is Gentle Shell's own `/gentle:changes` owner: the session's
//! roots (its repository and every worktree its subagents used), each
//! against its `HEAD`. Roots cross only as opaque ids with a directory-name
//! label, so the app never turns them into paths; a file's diff is read by
//! `(root, path)` exactly as listed. Files › Changes stays the repository's
//! own Git view; this is the agent's.
//!
//! The list is read once the probe lands and again whenever it may have
//! moved: a turn ended, a `session_snapshot` event carried the `changes`
//! section, events were lost, or the card opened. One request at a time on
//! the shared [`DesktopSession`]; a Shell without the methods (4.8.0) never
//! shows the card.

use std::rc::Rc;

use pi_rpc::UiRequest;
use pi_rpc::desktop_rpc::{
    Capabilities, ChangeDiff, ChangedFile, Changes, DesktopErrorCode, DesktopMethod, DesktopRecord,
    DesktopResponse, DesktopRpcError, DesktopSession,
};

use super::Send;

/// The event whose `changes` section says the change set moved.
const SESSION_SNAPSHOT: &str = "session_snapshot";

/// What one decision produced: the requests to send, and a failure the tab
/// shows through its report path.
#[derive(Debug, Default)]
pub(in crate::features::session) struct Applied {
    pub(in crate::features::session) sends: Vec<Send>,
    pub(in crate::features::session) report: Option<Report>,
}

/// Why a changes request stopped, for the conversation. What the server
/// answered about the change set itself (`changes_unavailable`, a file no
/// longer changed) is the card's to show, not the conversation's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::features::session) enum Report {
    /// The request timed out before the server answered.
    Timeout,
    /// A transport, server or decoding failure.
    Failure(DesktopRpcError),
}

impl Report {
    /// What the conversation says about it.
    pub(in crate::features::session) fn message(&self) -> String {
        match self {
            Self::Timeout => "Reading this session's changes timed out.".to_string(),
            Self::Failure(error) => {
                format!("This session's changes could not be read ({error}).")
            }
        }
    }
}

/// One file of the change set: its opaque root id and its path in that root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::features::session) struct FileKey {
    pub(in crate::features::session) root: String,
    pub(in crate::features::session) path: String,
}

impl FileKey {
    fn of(file: &ChangedFile) -> Self {
        Self {
            root: file.root.clone(),
            path: file.path.clone(),
        }
    }
}

/// The selected file's diff as far as the server answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::features::session) enum FileDiff {
    Loading,
    Ready(ChangeDiff),
    /// The server's path-free message (`changes_unavailable`, `not_found`).
    Failed(String),
}

#[derive(Debug, Default)]
pub(in crate::features::session) struct ChangeSet {
    capabilities: Option<Capabilities>,
    /// The last list the server answered.
    list: Option<Rc<Changes>>,
    /// The last read failed (`changes_unavailable`): the server's message.
    error: Option<String>,
    /// A `get_changes` is owed.
    stale: bool,
    /// The file whose diff the card shows.
    selected: Option<FileKey>,
    /// A `get_change_diff` is owed for `selected`.
    diff_due: bool,
    diff: Option<FileDiff>,
    /// Bumped whenever `diff` changes, so the view rebuilds its viewer once.
    diff_revision: u64,
    /// Bumped per request, so an old timer cannot drop a newer one.
    generation: u64,
}

impl ChangeSet {
    /// Adopt the peer's capabilities once the probe completes; the first
    /// read is owed from here.
    pub(in crate::features::session) fn set_capabilities(&mut self, capabilities: Capabilities) {
        self.capabilities = Some(capabilities);
        self.stale = self.supported();
    }

    /// Whether the peer offers both methods: a list without diffs is not
    /// worth a card next to Files › Changes.
    pub(in crate::features::session) fn supported(&self) -> bool {
        self.capabilities.as_ref().is_some_and(|capabilities| {
            capabilities.supports(DesktopMethod::GetChanges)
                && capabilities.supports(DesktopMethod::GetChangeDiff)
        })
    }

    /// The change set to show; `None` until read (or on an older Shell).
    /// Shared, so the card's render closure holds it without a copy.
    pub(in crate::features::session) fn list(&self) -> Option<&Rc<Changes>> {
        self.list.as_ref().filter(|_| self.supported())
    }

    /// Why the last read failed, if it did.
    pub(in crate::features::session) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(in crate::features::session) fn selected(&self) -> Option<&FileKey> {
        self.selected.as_ref()
    }

    pub(in crate::features::session) fn diff(&self) -> Option<&FileDiff> {
        self.diff.as_ref()
    }

    pub(in crate::features::session) fn diff_revision(&self) -> u64 {
        self.diff_revision
    }

    /// The list may have moved: read it again once the slot is free.
    pub(in crate::features::session) fn mark_stale(&mut self) {
        if self.supported() {
            self.stale = true;
        }
    }

    /// Show `file`'s diff, or close it when it is the one shown.
    pub(in crate::features::session) fn select(&mut self, file: Option<FileKey>) {
        if file.is_none() || file == self.selected {
            self.selected = None;
            self.diff_due = false;
            self.set_diff(None);
            return;
        }
        self.selected = file;
        self.diff_due = true;
        self.set_diff(Some(FileDiff::Loading));
    }

    /// Send what is owed once the slot is free: the diff the user is waiting
    /// for first, then the list.
    pub(in crate::features::session) fn pump(&mut self, session: &mut DesktopSession) -> Applied {
        if session.pending().is_some() || !self.supported() {
            return Applied::default();
        }
        let request = match &self.selected {
            Some(file) if self.diff_due => {
                self.diff_due = false;
                session.issue_get_change_diff(&file.root, &file.path)
            }
            _ if self.stale => {
                self.stale = false;
                session.issue(DesktopMethod::GetChanges)
            }
            _ => return Applied::default(),
        };
        match request {
            Ok(request) => {
                self.generation += 1;
                Applied {
                    sends: vec![Send {
                        request,
                        generation: self.generation,
                    }],
                    report: None,
                }
            }
            Err(error) => report(Report::Failure(error)),
        }
    }

    /// Pi answered the outer `prompt` of this slice's request.
    pub(in crate::features::session) fn outer(
        &mut self,
        session: &mut DesktopSession,
        generation: u64,
        handled: bool,
    ) -> Applied {
        if handled || generation != self.generation || !owns_pending(session) {
            return Applied::default();
        }
        self.rollback(session);
        report(Report::Failure(DesktopRpcError::UnexpectedDisposition {
            disposition: None,
        }))
    }

    /// The pending timeout for `generation` fired.
    pub(in crate::features::session) fn expire(
        &mut self,
        session: &mut DesktopSession,
        generation: u64,
    ) -> Applied {
        if generation != self.generation || !owns_pending(session) {
            return Applied::default();
        }
        self.rollback(session);
        report(Report::Timeout)
    }

    /// One incoming UI request; applies only this slice's two answers. It
    /// has no event: a `session_snapshot` with `changes` only marks it stale.
    pub(in crate::features::session) fn apply(
        &mut self,
        session: &mut DesktopSession,
        request: &UiRequest,
    ) -> Applied {
        let awaiting = session.pending().map(|(_, method)| method);
        match session.accept(request) {
            Ok(Some(DesktopRecord::Response(response))) => match awaiting {
                Some(DesktopMethod::GetChanges) => self.apply_list(response),
                Some(DesktopMethod::GetChangeDiff) => self.apply_diff(response),
                _ => Applied::default(),
            },
            _ => Applied::default(),
        }
    }

    /// The backend was replaced or exited: its roots and ids mean nothing
    /// now, and the next probe reads again.
    pub(in crate::features::session) fn reset(&mut self) {
        *self = Self {
            generation: self.generation + 1,
            diff_revision: self.diff_revision + 1,
            ..Self::default()
        };
    }

    /// Drop the request in flight. It is not retried in a loop: the next
    /// turn end, `changes` section or card open asks again.
    fn rollback(&mut self, session: &mut DesktopSession) {
        session.abandon();
        self.generation += 1;
        if matches!(self.diff, Some(FileDiff::Loading)) {
            self.set_diff(Some(FileDiff::Failed(
                "Gentle Shell did not answer in time.".into(),
            )));
        }
    }

    fn apply_list(&mut self, response: DesktopResponse) -> Applied {
        match response.changes() {
            Ok(list) => {
                self.error = None;
                // A file that is no longer changed closes; one whose counts
                // moved is read again.
                if let Some(selected) = &self.selected {
                    let before = self.find(selected).cloned();
                    match list
                        .files
                        .iter()
                        .find(|file| FileKey::of(file) == *selected)
                    {
                        None => {
                            self.selected = None;
                            self.diff_due = false;
                            self.set_diff(None);
                        }
                        Some(now) if before.as_ref() != Some(now) => self.diff_due = true,
                        Some(_) => {}
                    }
                }
                self.list = Some(Rc::new(list));
                Applied::default()
            }
            Err(DesktopRpcError::RemoteError(info))
                if info.kind() == Some(DesktopErrorCode::ChangesUnavailable) =>
            {
                // A failed scan is never a clean session: say so, keep nothing.
                self.list = None;
                self.error = Some(info.message);
                Applied::default()
            }
            Err(error) => report(Report::Failure(error)),
        }
    }

    fn apply_diff(&mut self, response: DesktopResponse) -> Applied {
        let shown = match response.change_diff() {
            Ok(diff) => {
                let key = FileKey {
                    root: diff.root.clone(),
                    path: diff.path.clone(),
                };
                if self.selected.as_ref() != Some(&key) {
                    // The user moved on while it was on the wire.
                    return Applied::default();
                }
                FileDiff::Ready(diff)
            }
            Err(DesktopRpcError::RemoteError(info)) => {
                if info.kind() == Some(DesktopErrorCode::NotFound) {
                    // No longer in the change set: the list is behind.
                    self.mark_stale();
                }
                FileDiff::Failed(info.message)
            }
            Err(error) => return report(Report::Failure(error)),
        };
        if self.selected.is_some() {
            self.set_diff(Some(shown));
        }
        Applied::default()
    }

    fn find(&self, key: &FileKey) -> Option<&ChangedFile> {
        self.list
            .as_ref()?
            .files
            .iter()
            .find(|file| FileKey::of(file) == *key)
    }

    fn set_diff(&mut self, diff: Option<FileDiff>) {
        if self.diff != diff {
            self.diff = diff;
            self.diff_revision += 1;
        }
    }
}

/// Whether a decoded record is a `session_snapshot` event carrying the
/// `changes` section, i.e. the change set moved. Its content is the feed's
/// path-free summary; the list itself is still read with `get_changes`.
pub(in crate::features::session) fn changes_moved(
    decoded: &Result<Option<DesktopRecord>, DesktopRpcError>,
) -> bool {
    matches!(decoded, Ok(Some(DesktopRecord::Event(event)))
        if event.event == SESSION_SNAPSHOT
            && event
                .session_snapshot()
                .is_ok_and(|snapshot| snapshot.sections.changes.is_some()))
}

fn report(report: Report) -> Applied {
    Applied {
        sends: Vec::new(),
        report: Some(report),
    }
}

/// Whether the pending request is this slice's: the other slices share the
/// session, so a stale timer here must not drop theirs.
fn owns_pending(session: &DesktopSession) -> bool {
    matches!(
        session.pending().map(|(_, method)| method),
        Some(DesktopMethod::GetChanges | DesktopMethod::GetChangeDiff)
    )
}

#[cfg(test)]
mod tests {
    use pi_rpc::desktop_rpc::{
        Capabilities, DesktopMethod, DesktopSession, PROMPT_COMMAND, SCHEMA, WIDGET_KEY,
    };
    use pi_rpc::{UiMethod, UiRequest};
    use serde_json::{Value, json};

    use super::{ChangeSet, FileDiff, FileKey, Report};

    fn widget(record: Value) -> UiRequest {
        UiRequest {
            id: "ui-1".into(),
            timeout: None,
            method: UiMethod::SetWidget {
                key: WIDGET_KEY.into(),
                lines: Some(vec![record.to_string()]),
                placement: None,
            },
        }
    }

    fn capabilities(methods: &[&str]) -> Capabilities {
        Capabilities {
            methods: methods.iter().map(|name| (*name).to_string()).collect(),
            events: Vec::new(),
            schema: Some(SCHEMA.into()),
            command: Some(PROMPT_COMMAND.into()),
            widget: Some(WIDGET_KEY.into()),
        }
    }

    fn ok(request_id: &str, data: Value) -> UiRequest {
        widget(json!({
            "schema": SCHEMA, "kind": "response", "requestId": request_id, "ok": true, "data": data,
        }))
    }

    fn error(request_id: &str, code: &str, message: &str) -> UiRequest {
        widget(json!({
            "schema": SCHEMA, "kind": "response", "requestId": request_id, "ok": false,
            "error": {"code": code, "message": message},
        }))
    }

    fn send(changes: &mut ChangeSet, session: &mut DesktopSession) -> (String, Value) {
        let applied = changes.pump(session);
        assert_eq!(applied.sends.len(), 1, "one request");
        let request = &applied.sends[0].request;
        let payload: Value = serde_json::from_str(request.payload()).unwrap();
        assert!(matches!(
            payload["method"].as_str(),
            Some("get_changes" | "get_change_diff")
        ));
        (request.request_id().to_string(), payload["params"].clone())
    }

    fn list(notes_added: u64) -> Value {
        json!({
            "roots": [{"id": "root-1", "label": "workspace"}],
            "files": [
                {"root": "root-1", "path": "notes.txt", "status": "modified", "added": notes_added, "deleted": 1},
                {"root": "root-1", "path": "fresh.txt", "status": "untracked", "added": 3, "deleted": 0},
            ],
            "truncated": false,
        })
    }

    fn key(path: &str) -> FileKey {
        FileKey {
            root: "root-1".into(),
            path: path.into(),
        }
    }

    fn ready() -> (ChangeSet, DesktopSession) {
        let mut changes = ChangeSet::default();
        changes.set_capabilities(capabilities(&["get_changes", "get_change_diff"]));
        let mut session = DesktopSession::new();
        let (id, params) = send(&mut changes, &mut session);
        assert_eq!(params, json!({}));
        let _ = changes.apply(&mut session, &ok(&id, list(2)));
        (changes, session)
    }

    #[test]
    fn a_shell_without_the_methods_shows_and_sends_nothing() {
        let mut changes = ChangeSet::default();
        changes.set_capabilities(capabilities(&["get_changes"]));
        let mut session = DesktopSession::new();
        changes.mark_stale();
        changes.select(Some(key("notes.txt")));
        assert!(changes.pump(&mut session).sends.is_empty());
        assert!(changes.list().is_none());
        assert!(session.pending().is_none());
    }

    #[test]
    fn the_list_is_read_once_then_again_only_when_stale() {
        let (mut changes, mut session) = ready();
        let shown = changes.list().unwrap();
        assert_eq!(shown.roots[0].label, "workspace");
        assert_eq!(shown.files.len(), 2);
        assert!(changes.pump(&mut session).sends.is_empty(), "read once");

        changes.mark_stale();
        let _busy = session.issue(DesktopMethod::GetSnapshot).unwrap();
        assert!(
            changes.pump(&mut session).sends.is_empty(),
            "waits for the slot"
        );
        session.abandon();
        let (id, _) = send(&mut changes, &mut session);
        let _ = changes.apply(
            &mut session,
            &ok(&id, json!({"roots": [], "files": [], "truncated": false})),
        );
        assert!(changes.list().unwrap().files.is_empty());
    }

    #[test]
    fn a_selected_file_reads_its_diff_by_root_and_path() {
        let (mut changes, mut session) = ready();
        changes.select(Some(key("notes.txt")));
        assert_eq!(changes.diff(), Some(&FileDiff::Loading));
        let (id, params) = send(&mut changes, &mut session);
        assert_eq!(params, json!({"root": "root-1", "path": "notes.txt"}));
        let before = changes.diff_revision();
        let _ = changes.apply(
            &mut session,
            &ok(
                &id,
                json!({"root": "root-1", "path": "notes.txt", "diff": "+two\n", "truncated": false}),
            ),
        );
        assert!(matches!(changes.diff(), Some(FileDiff::Ready(diff)) if diff.diff == "+two\n"));
        assert!(changes.diff_revision() > before);

        // The same file again closes it.
        changes.select(Some(key("notes.txt")));
        assert!(changes.selected().is_none() && changes.diff().is_none());
    }

    #[test]
    fn a_diff_for_a_file_the_user_left_is_dropped() {
        let (mut changes, mut session) = ready();
        changes.select(Some(key("notes.txt")));
        let (id, _) = send(&mut changes, &mut session);
        changes.select(Some(key("fresh.txt")));
        let _ = changes.apply(
            &mut session,
            &ok(
                &id,
                json!({"root": "root-1", "path": "notes.txt", "diff": "x", "truncated": false}),
            ),
        );
        assert_eq!(changes.diff(), Some(&FileDiff::Loading));
        let (_, params) = send(&mut changes, &mut session);
        assert_eq!(params["path"], "fresh.txt");
    }

    #[test]
    fn a_refresh_rereads_a_moved_diff_and_closes_a_vanished_file() {
        let (mut changes, mut session) = ready();
        changes.select(Some(key("notes.txt")));
        let (id, _) = send(&mut changes, &mut session);
        let _ = changes.apply(
            &mut session,
            &ok(
                &id,
                json!({"root": "root-1", "path": "notes.txt", "diff": "+two\n", "truncated": false}),
            ),
        );
        // Same counts: nothing to re-read.
        changes.mark_stale();
        let (id, _) = send(&mut changes, &mut session);
        let _ = changes.apply(&mut session, &ok(&id, list(2)));
        assert!(changes.pump(&mut session).sends.is_empty());
        // The agent edited it again: its diff is read again.
        changes.mark_stale();
        let (id, _) = send(&mut changes, &mut session);
        let _ = changes.apply(&mut session, &ok(&id, list(5)));
        let (_, params) = send(&mut changes, &mut session);
        assert_eq!(params["path"], "notes.txt");
    }

    #[test]
    fn server_answers_about_the_change_set_stay_on_the_card() {
        let (mut changes, mut session) = ready();
        changes.select(Some(key("notes.txt")));
        let (id, _) = send(&mut changes, &mut session);
        let applied = changes.apply(
            &mut session,
            &error(&id, "not_found", "No changed file with that root and path"),
        );
        assert!(applied.report.is_none());
        assert_eq!(
            changes.diff(),
            Some(&FileDiff::Failed(
                "No changed file with that root and path".into()
            ))
        );
        // The list was behind: read again.
        let (id, params) = send(&mut changes, &mut session);
        assert_eq!(params, json!({}));
        let applied = changes.apply(
            &mut session,
            &error(&id, "changes_unavailable", "Git failed for a root"),
        );
        assert!(applied.report.is_none());
        assert!(changes.list().is_none(), "a failed scan is not a clean one");
        assert_eq!(changes.error(), Some("Git failed for a root"));
    }

    #[test]
    fn a_timeout_reports_and_a_stale_timer_or_late_answer_is_benign() {
        let (mut changes, mut session) = ready();
        changes.select(Some(key("notes.txt")));
        let generation = changes.pump(&mut session).sends[0].generation;
        assert!(
            changes
                .expire(&mut session, generation + 1)
                .report
                .is_none()
        );
        assert_eq!(
            changes.expire(&mut session, generation).report,
            Some(Report::Timeout)
        );
        assert!(matches!(changes.diff(), Some(FileDiff::Failed(_))));
        assert!(session.pending().is_none());

        changes.reset();
        session.reset();
        assert!(changes.list().is_none() && changes.selected().is_none());
        let applied = changes.apply(&mut session, &ok("desktop-1", list(2)));
        assert!(applied.report.is_none());
        assert!(changes.list().is_none());
    }
}
