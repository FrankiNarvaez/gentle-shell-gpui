//! The composer's prompt history, as in Gentle Shell's terminal: ↑ recalls
//! the prompts sent before, newest first, and ↓ walks back toward the
//! newest, then to the draft that was being written. Recalling never sends.

/// The most prompts a tab remembers.
pub const LIMIT: usize = 200;

/// Sent prompts, oldest first, and where ↑/↓ are in them.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct PromptHistory {
    entries: Vec<String>,
    /// The entry the composer shows; `None` while it shows the draft.
    at: Option<usize>,
    /// What the composer held when ↑ first left it.
    draft: String,
}

impl PromptHistory {
    /// Remember a prompt just sent, and leave the history: the next ↑
    /// starts from it. Blank prompts and a repeat of the newest are skipped.
    pub fn record(&mut self, prompt: &str) {
        self.leave();
        let prompt = prompt.trim();
        if prompt.is_empty() || self.entries.last().is_some_and(|last| last == prompt) {
            return;
        }
        self.entries.push(prompt.to_string());
        if self.entries.len() > LIMIT {
            self.entries.remove(0);
        }
    }

    /// Start from a resumed conversation's prompts (oldest first) when the
    /// tab has none of its own yet. Later loads of the same conversation
    /// (a restart, a resync) change nothing: what this tab sent is in them.
    pub fn seed<'a>(&mut self, prompts: impl IntoIterator<Item = &'a str>) {
        if !self.entries.is_empty() {
            return;
        }
        for prompt in prompts {
            self.record(prompt);
        }
    }

    /// ↑: the prompt before the one shown, keeping `current` as the draft
    /// when it leaves it. `None` when there is nothing older.
    pub fn older(&mut self, current: &str) -> Option<&str> {
        let at = match self.at {
            None if self.entries.is_empty() => return None,
            None => {
                self.draft = current.to_string();
                self.entries.len() - 1
            }
            Some(0) => return None,
            Some(at) => at - 1,
        };
        self.at = Some(at);
        Some(&self.entries[at])
    }

    /// ↓: the prompt after the one shown, or the draft past the newest.
    /// `None` while the draft is already shown.
    pub fn newer(&mut self) -> Option<&str> {
        let at = self.at?;
        if at + 1 < self.entries.len() {
            self.at = Some(at + 1);
            Some(&self.entries[at + 1])
        } else {
            self.at = None;
            Some(&self.draft)
        }
    }

    /// Whether the composer shows a recalled prompt rather than the draft.
    pub fn browsing(&self) -> bool {
        self.at.is_some()
    }

    /// Back to the draft without touching the composer.
    pub fn leave(&mut self) {
        self.at = None;
        self.draft.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(prompts: &[&str]) -> PromptHistory {
        let mut history = PromptHistory::default();
        for prompt in prompts {
            history.record(prompt);
        }
        history
    }

    #[test]
    fn up_recalls_newest_first_and_down_returns_to_the_draft() {
        let mut h = history(&["first", "second", "third"]);
        assert_eq!(h.older("half typed"), Some("third"));
        assert_eq!(h.older("third"), Some("second"));
        assert_eq!(h.older("second"), Some("first"));
        assert_eq!(h.older("first"), None, "nothing older than the oldest");
        assert!(h.browsing());
        assert_eq!(h.newer(), Some("second"));
        assert_eq!(h.newer(), Some("third"));
        assert_eq!(h.newer(), Some("half typed"), "the draft comes back");
        assert!(!h.browsing());
        assert_eq!(h.newer(), None);
    }

    #[test]
    fn an_empty_history_leaves_the_composer_alone() {
        let mut h = PromptHistory::default();
        assert_eq!(h.older("draft"), None);
        assert!(!h.browsing());
        assert_eq!(h.newer(), None);
    }

    #[test]
    fn sending_records_once_and_starts_over_from_the_newest() {
        let mut h = history(&["fix it", "  ", "fix it", "test it"]);
        assert_eq!(h.older(""), Some("test it"));
        assert_eq!(h.older(""), Some("fix it"));
        assert_eq!(h.older(""), None, "blank and repeated prompts are skipped");
        h.record("ship it");
        assert!(!h.browsing());
        assert_eq!(h.older(""), Some("ship it"));
    }

    #[test]
    fn a_resumed_conversation_seeds_only_an_empty_history() {
        let mut h = PromptHistory::default();
        h.seed(["old one", "old one", "", "older two"]);
        assert_eq!(h.older(""), Some("older two"));
        assert_eq!(h.older(""), Some("old one"));
        assert_eq!(h.older(""), None);
        // Loaded again (a restart): what the tab already has stays as is.
        h.seed(["old one", "older two", "new one"]);
        assert_eq!(h.newer(), Some("older two"));
        assert_eq!(h.newer(), Some(""));
    }

    #[test]
    fn it_keeps_only_the_newest_prompts() {
        let prompts: Vec<String> = (0..LIMIT + 5).map(|i| format!("p{i}")).collect();
        let mut h = PromptHistory::default();
        for prompt in &prompts {
            h.record(prompt);
        }
        let mut oldest = None;
        while let Some(prompt) = h.older("") {
            oldest = Some(prompt.to_string());
        }
        assert_eq!(oldest.as_deref(), Some("p5"));
    }
}
