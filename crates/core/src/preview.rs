//! How much of a file or a diff the file pane loads. Up to its soft limit it
//! opens at once; past it the pane asks first (Load anyway); past the hard
//! limit it never loads, since the editor and the diff hold many times the
//! text in memory. Large content shows without syntax colors: the parse
//! behind them takes memory and background work in proportion to its size.

const MB: u64 = 1024 * 1024;

/// What a file or diff of some size gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// Loads at once.
    Load,
    /// Over the soft limit: loads once the user asks.
    Ask,
    /// Over the hard limit: doesn't load.
    Refuse,
}

/// A soft and a hard size limit, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub soft: u64,
    pub hard: u64,
}

impl Limits {
    /// What content of `size` bytes gets; `asked` once the user chose to
    /// load it past the soft limit.
    pub fn fit(self, size: u64, asked: bool) -> Fit {
        if size > self.hard {
            Fit::Refuse
        } else if size > self.soft && !asked {
            Fit::Ask
        } else {
            Fit::Load
        }
    }
}

/// A file shown in the editor. 8 MB of text opens in about 50 ms of UI
/// time (most of it the editor's first layout) and takes about 100 MB.
pub const FILE: Limits = Limits {
    soft: 8 * MB,
    hard: 64 * MB,
};

/// A diff, measured as git's patch text (an untracked file: its size). The
/// diff view holds about 20 times the patch in memory, 40 with colors.
pub const PATCH: Limits = Limits {
    soft: 4 * MB,
    hard: 64 * MB,
};

/// Content up to this size is shown with syntax colors: past it they'd
/// double a diff's memory and add half again to a file's.
pub const COLOR_LIMIT: u64 = 8 * MB;

/// Whether content of `size` bytes is shown with syntax colors.
pub fn colors(size: u64) -> bool {
    size <= COLOR_LIMIT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_up_to_the_soft_limit_loads_at_once() {
        assert_eq!(FILE.fit(0, false), Fit::Load);
        assert_eq!(FILE.fit(FILE.soft, false), Fit::Load);
        assert_eq!(PATCH.fit(PATCH.soft, false), Fit::Load);
    }

    #[test]
    fn past_the_soft_limit_it_asks_and_loads_once_asked() {
        assert_eq!(FILE.fit(FILE.soft + 1, false), Fit::Ask);
        assert_eq!(FILE.fit(FILE.soft + 1, true), Fit::Load);
        assert_eq!(FILE.fit(FILE.hard, true), Fit::Load);
        assert_eq!(PATCH.fit(PATCH.soft + 1, false), Fit::Ask);
    }

    #[test]
    fn past_the_hard_limit_it_never_loads() {
        assert_eq!(FILE.fit(FILE.hard + 1, false), Fit::Refuse);
        assert_eq!(FILE.fit(FILE.hard + 1, true), Fit::Refuse);
        assert_eq!(PATCH.fit(PATCH.hard + 1, true), Fit::Refuse);
    }

    #[test]
    fn large_content_shows_without_colors() {
        assert!(colors(FILE.soft));
        assert!(colors(COLOR_LIMIT));
        assert!(!colors(COLOR_LIMIT + 1));
    }
}
