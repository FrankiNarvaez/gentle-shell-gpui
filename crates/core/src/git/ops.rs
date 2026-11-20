//! The Git pane's operations: the arguments each one hands `git`, the
//! sequence git stops in when one conflicts, and a commit's changed files.

use std::collections::BTreeMap;

use super::status::parse_numstat;

/// How Pull brings the upstream's commits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PullMode {
    /// Fast-forward when possible, else a merge commit.
    #[default]
    Merge,
    /// Only a fast-forward; refuses when the branches diverged.
    FastForwardOnly,
    /// Replay local commits on top of the upstream.
    Rebase,
}

impl PullMode {
    pub const ALL: [PullMode; 3] = [PullMode::Merge, PullMode::FastForwardOnly, PullMode::Rebase];

    pub fn label(self) -> &'static str {
        match self {
            PullMode::Merge => "Fast-forward if possible",
            PullMode::FastForwardOnly => "Fast-forward only",
            PullMode::Rebase => "Rebase",
        }
    }

    /// `git pull` with this mode; never opens an editor.
    pub fn args(self) -> &'static [&'static str] {
        match self {
            PullMode::Merge => &["pull", "--no-rebase", "--no-edit"],
            PullMode::FastForwardOnly => &["pull", "--ff-only"],
            PullMode::Rebase => &["pull", "--rebase"],
        }
    }
}

/// What Reset keeps when it moves the current branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetMode {
    /// The commits' changes and the work tree stay, unstaged (`--mixed`).
    KeepChanges,
    /// The work tree and the index match the commit (`--hard`).
    DiscardChanges,
}

impl ResetMode {
    pub fn flag(self) -> &'static str {
        match self {
            ResetMode::KeepChanges => "--mixed",
            ResetMode::DiscardChanges => "--hard",
        }
    }
}

/// A command git stopped in the middle of, waiting on the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InProgress {
    Rebase,
    Merge,
    CherryPick,
    Revert,
}

impl InProgress {
    /// What git leaves in its directory while each one is under way. A
    /// rebase is asked first: it may also leave `CHERRY_PICK_HEAD` behind.
    const MARKERS: [(&'static str, InProgress); 5] = [
        ("rebase-merge", InProgress::Rebase),
        ("rebase-apply", InProgress::Rebase),
        ("MERGE_HEAD", InProgress::Merge),
        ("CHERRY_PICK_HEAD", InProgress::CherryPick),
        ("REVERT_HEAD", InProgress::Revert),
    ];

    /// The command in progress, from which entries exist in the git
    /// directory.
    pub fn detect(exists: impl Fn(&str) -> bool) -> Option<InProgress> {
        Self::MARKERS
            .iter()
            .find(|(name, _)| exists(name))
            .map(|(_, op)| *op)
    }

    pub fn label(self) -> &'static str {
        match self {
            InProgress::Rebase => "Rebase",
            InProgress::Merge => "Merge",
            InProgress::CherryPick => "Cherry-pick",
            InProgress::Revert => "Revert",
        }
    }

    fn command(self) -> &'static str {
        match self {
            InProgress::Rebase => "rebase",
            InProgress::Merge => "merge",
            InProgress::CherryPick => "cherry-pick",
            InProgress::Revert => "revert",
        }
    }

    /// Carry on once the conflicts are resolved and staged. The editor is
    /// `true`, so git keeps the message it prepared.
    pub fn continue_args(self) -> Vec<&'static str> {
        vec!["-c", "core.editor=true", self.command(), "--continue"]
    }

    /// Give up and go back to where it started.
    pub fn abort_args(self) -> Vec<&'static str> {
        vec![self.command(), "--abort"]
    }
}

/// A file a commit changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitFile {
    pub path: String,
    /// The path before a rename or copy.
    pub old_path: Option<String>,
    /// `A`, `M`, `D`, `R`, `C`, `T`.
    pub status: char,
    /// `None` for binary files.
    pub added: Option<u64>,
    pub removed: Option<u64>,
}

/// `git diff-tree` arguments for a commit's changes against its first
/// parent (`parent`), or against nothing for a root commit. `kind` is
/// `--name-status` or `--numstat`.
pub fn diff_tree_args<'a>(kind: &'a str, hash: &'a str, parent: Option<&'a str>) -> Vec<&'a str> {
    let mut args = vec!["diff-tree", "-r", "-M", "-z", "--no-commit-id", kind];
    match parent {
        Some(parent) => args.extend([parent, hash]),
        None => args.extend(["--root", hash]),
    }
    args
}

/// Join `diff-tree -z --name-status` and `diff-tree -z --numstat` output.
pub fn parse_commit_files(name_status: &str, numstat: &str) -> Vec<CommitFile> {
    let counts: BTreeMap<String, (Option<u64>, Option<u64>)> = parse_numstat(numstat);
    let mut fields = name_status.split('\0').filter(|f| !f.is_empty());
    let mut files = Vec::new();
    while let Some(code) = fields.next() {
        let Some(status) = code.chars().next() else {
            continue;
        };
        // Renames and copies name the old path, then the new one.
        let (old_path, path) = if matches!(status, 'R' | 'C') {
            let old = fields.next();
            (old.map(str::to_string), fields.next())
        } else {
            (None, fields.next())
        };
        let Some(path) = path else { break };
        let (added, removed) = counts.get(path).copied().unwrap_or((None, None));
        files.push(CommitFile {
            path: path.to_string(),
            old_path,
            status,
            added,
            removed,
        });
    }
    files
}

/// What the agent is asked when the user wants a commit explained.
pub fn explain_prompt(short: &str, subject: &str) -> String {
    format!(
        "Explain commit {short} (\"{subject}\") in this repository: what it changes and why. \
         Read it with `git show {short}` and the code around it. Don't change any files."
    )
}

/// `message` with its first line replaced by `subject`; the body stays.
pub fn replace_subject(message: &str, subject: &str) -> String {
    let body = message.split_once('\n').map_or("", |(_, rest)| rest).trim();
    if body.is_empty() {
        subject.trim().to_string()
    } else {
        format!("{}\n\n{body}", subject.trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_progress_prefers_rebase_and_reads_each_marker() {
        assert_eq!(InProgress::detect(|_| false), None);
        assert_eq!(
            InProgress::detect(|n| n == "MERGE_HEAD"),
            Some(InProgress::Merge)
        );
        // A rebase that stopped on a picked commit leaves CHERRY_PICK_HEAD too.
        assert_eq!(
            InProgress::detect(|n| n == "CHERRY_PICK_HEAD" || n == "rebase-merge"),
            Some(InProgress::Rebase)
        );
        assert_eq!(
            InProgress::detect(|n| n == "REVERT_HEAD"),
            Some(InProgress::Revert)
        );
    }

    #[test]
    fn replacing_the_subject_keeps_the_body() {
        assert_eq!(
            replace_subject(
                "old\n\nWhy it changed.\n\nCo-authored-by: X <x@y>\n",
                " new "
            ),
            "new\n\nWhy it changed.\n\nCo-authored-by: X <x@y>"
        );
        assert_eq!(replace_subject("old", "new"), "new");
    }

    #[test]
    fn commit_files_join_statuses_renames_and_counts() {
        let name_status = "M\0src/a.rs\0R087\0old/b.rs\0new/b.rs\0A\0logo.png\0D\0gone.txt\0";
        let numstat =
            "3\t1\tsrc/a.rs\x002\t2\t\0old/b.rs\0new/b.rs\0-\t-\tlogo.png\x000\t9\tgone.txt\0";
        let files = parse_commit_files(name_status, numstat);
        assert_eq!(files.len(), 4);
        assert_eq!(
            files[0],
            CommitFile {
                path: "src/a.rs".into(),
                old_path: None,
                status: 'M',
                added: Some(3),
                removed: Some(1),
            }
        );
        assert_eq!(files[1].status, 'R');
        assert_eq!(files[1].old_path.as_deref(), Some("old/b.rs"));
        assert_eq!(files[1].path, "new/b.rs");
        assert_eq!((files[1].added, files[1].removed), (Some(2), Some(2)));
        // Binary: no counts.
        assert_eq!((files[2].added, files[2].removed), (None, None));
        assert_eq!(files[3].status, 'D');
        // Truncated output stops instead of inventing a file.
        assert!(parse_commit_files("M\0", "").is_empty());
    }
}
