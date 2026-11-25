//! What a prompt carries besides its typed text, as far as that is rules
//! only: regular files named by path in the prompt, long pasted texts and the
//! `[Pasted text #N]` composer tokens that stand for them, and the `[Image #N]`
//! tokens of messages sent before images left the composer. Reading a path,
//! sniffing and encoding an image and decoding conversation images need GPUI
//! and the filesystem: they stay in the app
//! (`features/session/attachments.rs`), which calls this module.

use std::fmt;
use std::ops::Range;
use std::path::{Path, PathBuf};

// ── Files ──────────────────────────────────────────────────────────────────

/// A regular file waiting in the composer, sent to the agent by path: the
/// agent reads it with its own tools, nothing of it is inlined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachedFile {
    /// The file's name, as its card and the sent prompt name it.
    pub name: String,
    /// The file as it was dropped or picked.
    pub path: PathBuf,
    /// Its size in bytes, as its card shows it.
    pub bytes: u64,
}

/// Why a file was not attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    /// A folder: only files attach.
    Folder {
        name: String,
    },
    Unsupported {
        name: String,
    },
    TooLarge {
        name: String,
        bytes: u64,
    },
    Unreadable {
        name: String,
        error: String,
    },
}

impl fmt::Display for Rejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Folder { name } => write!(f, "{name} is a folder; attach files instead."),
            Self::Unsupported { name } => {
                write!(
                    f,
                    "{name} is not a PNG, JPEG, GIF, WebP, BMP or TIFF image."
                )
            }
            Self::TooLarge { name, bytes } => write!(
                f,
                "{name} is {:.1} MB; images must be 20 MB or smaller.",
                *bytes as f64 / (1024.0 * 1024.0)
            ),
            Self::Unreadable { name, error } => write!(f, "Could not read {name}: {error}"),
        }
    }
}

/// Where a prompt names `path`: relative to `cwd` with `/` separators when it
/// is inside it, else absolute.
fn message_path(path: &Path, cwd: &Path) -> String {
    match path.strip_prefix(cwd) {
        Ok(rest) if !rest.as_os_str().is_empty() => rest
            .components()
            .map(|part| part.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
        _ => path.display().to_string(),
    }
}

/// `text` followed by the block naming `files`, one path per line; the block
/// alone when `text` is blank. Contents stay out: the agent reads them.
pub fn with_files(text: &str, files: &[AttachedFile], cwd: &Path) -> String {
    if files.is_empty() {
        return text.to_string();
    }
    let block = files
        .iter()
        .map(|file| format!("- {}", message_path(&file.path, cwd)))
        .collect::<Vec<_>>()
        .join("\n");
    if text.trim().is_empty() {
        format!("Attached files:\n{block}")
    } else {
        format!("{text}\n\nAttached files:\n{block}")
    }
}

/// A file's size as its card shows it: "12 B", "2.0 KB", "3.0 MB".
pub fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let size = bytes as f64;
    if size < KB {
        format!("{bytes} B")
    } else if size < KB * KB {
        format!("{:.1} KB", size / KB)
    } else {
        format!("{:.1} MB", size / KB / KB)
    }
}

// ── Tokens ─────────────────────────────────────────────────────────────────

/// The composer token pasted text `n` (1-based) stands for. Images have no
/// token: they wait in the strip above the composer, numbered there.
pub fn token(n: usize) -> String {
    format!("[Pasted text #{n}]")
}

/// Whether a token inserted after `before` needs a space ahead of it.
pub fn needs_lead(before: Option<char>) -> bool {
    before.is_some_and(|c| !c.is_whitespace())
}

/// `[` and the name: where a paste's token starts.
const OPEN: &str = "[Pasted text";

/// Where an image token starts in messages sent before images left the
/// composer.
const IMAGE_OPEN: &str = "[Image";

/// Every paste token in `text`, spelled `[Pasted text #N]` or
/// `[Pasted text#N]`: byte range and number.
pub fn find_tokens(text: &str) -> Vec<(Range<usize>, usize)> {
    find_named(text, OPEN)
}

/// A sent message's text with each old `[Image #N]` token shown as `🖼 #N`:
/// sessions from before images left the composer still carry them.
pub fn chip_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (range, number) in find_named(text, IMAGE_OPEN) {
        out.push_str(&text[last..range.start]);
        out.push_str(&format!("🖼 #{number}"));
        last = range.end;
    }
    out.push_str(&text[last..]);
    out
}

/// Every token opened by `open` (`[` and its name), spelled `{open} #N]` or
/// `{open}#N]`: byte range and number.
fn find_named(text: &str, open: &str) -> Vec<(Range<usize>, usize)> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(offset) = text[from..].find(open) {
        let start = from + offset;
        let mut rest = &text[start + open.len()..];
        rest = rest.strip_prefix(' ').unwrap_or(rest);
        let parsed = rest.strip_prefix('#').and_then(|rest| {
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            let number = rest[..digits].parse::<usize>().ok()?;
            rest[digits..].starts_with(']').then_some((number, digits))
        });
        match parsed {
            Some((number, digits)) => {
                let end = text.len() - rest.len() + 1 + digits + 1;
                found.push((start..end, number));
                from = end;
            }
            None => from = start + open.len(),
        }
    }
    found
}

/// `text` after paste `removed` is dropped: its token goes (with one
/// adjoining space) and later tokens shift down so `#N` stays the N-th one.
pub fn remove_token(text: &str, removed: usize) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (range, number) in find_tokens(text) {
        out.push_str(&text[last..range.start]);
        last = range.end;
        if number == removed {
            if text[last..].starts_with(' ') {
                last += 1;
            } else if out.ends_with(' ') {
                out.pop();
            }
        } else if number > removed {
            out.push_str(&token(number - 1));
        } else {
            out.push_str(&text[range]);
        }
    }
    out.push_str(&text[last..]);
    out
}

/// Which of the `count` pastes still have a token in `text`: their numbers,
/// ascending, each once.
pub fn present(text: &str, count: usize) -> Vec<usize> {
    let mut numbers: Vec<usize> = find_tokens(text)
        .into_iter()
        .map(|(_, number)| number)
        .filter(|number| (1..=count).contains(number))
        .collect();
    numbers.sort_unstable();
    numbers.dedup();
    numbers
}

/// `text` once only the pastes `kept` (ascending, from [`present`]) remain:
/// the i-th kept one's tokens become `#i`. Numbers not kept (typed past the
/// last paste) stay as typed.
pub fn renumber(text: &str, kept: &[usize]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (range, number) in find_tokens(text) {
        out.push_str(&text[last..range.start]);
        match kept.binary_search(&number) {
            Ok(ix) => out.push_str(&token(ix + 1)),
            Err(_) => out.push_str(&text[range.clone()]),
        }
        last = range.end;
    }
    out.push_str(&text[last..]);
    out
}

// ── Pasted text ────────────────────────────────────────────────────────────

/// A paste of at least this many lines, or this many characters, is kept
/// as a `[Pasted text #N]` attachment instead of being inserted: below
/// either, it pastes as text.
const PASTE_LINES: usize = 6;
const PASTE_CHARS: usize = 800;

/// Lines and columns of a pasted text its card's tile shows.
const PREVIEW_LINES: usize = 8;
const PREVIEW_COLUMNS: usize = 24;

/// A long paste waiting in the composer, sent in place of its token.
#[derive(Clone, Debug)]
pub struct PastedText {
    /// Exactly what was pasted.
    pub text: String,
    /// [`line_count`] of `text`.
    pub lines: usize,
    /// The first lines, cut short, for the card's tile.
    pub preview: String,
}

impl PastedText {
    pub fn new(text: String) -> Self {
        Self {
            lines: line_count(&text),
            preview: preview(&text),
            text,
        }
    }
}

/// Lines in `text`: `\n` and `\r\n` both end one, and a final line break
/// does not start another.
fn line_count(text: &str) -> usize {
    text.lines().count()
}

/// Whether a pasted `text` is long enough to become an attachment (see
/// [`PASTE_LINES`]). Blank pastes never are.
pub fn is_long_paste(text: &str) -> bool {
    !text.trim().is_empty()
        && (text.lines().nth(PASTE_LINES - 1).is_some()
            || text.chars().nth(PASTE_CHARS - 1).is_some())
}

/// The first lines of `text`, each cut to a few columns, tabs as spaces.
fn preview(text: &str) -> String {
    text.lines()
        .take(PREVIEW_LINES)
        .map(|line| {
            line.chars()
                .take(PREVIEW_COLUMNS)
                .collect::<String>()
                .replace('\t', "  ")
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `text` with each `[Pasted text #N]` replaced by the N-th of `pasted`,
/// verbatim. Tokens past the last paste stay as typed, and tokens inside a
/// pasted text are its own content: they are not expanded again.
pub fn expand(text: &str, pasted: &[PastedText]) -> String {
    let tokens = find_tokens(text);
    let pasted_len: usize = pasted.iter().map(|p| p.text.len()).sum();
    let mut out = String::with_capacity(text.len() + pasted_len);
    let mut last = 0;
    for (range, number) in tokens {
        let Some(paste) = number.checked_sub(1).and_then(|ix| pasted.get(ix)) else {
            continue;
        };
        out.push_str(&text[last..range.start]);
        out.push_str(&paste.text);
        last = range.end;
    }
    out.push_str(&text[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file as a prompt names it; only its path matters to the block.
    fn file(path: &str) -> AttachedFile {
        AttachedFile {
            name: path.rsplit('/').next().unwrap().to_string(),
            path: PathBuf::from(path),
            bytes: 0,
        }
    }

    /// S4: files are sent as paths in a short block after the text, relative
    /// to the agent's folder when they are inside it, else absolute; a
    /// prompt with only files is the block alone.
    #[test]
    fn files_are_sent_as_paths_after_the_text() {
        let cwd = Path::new("/work");
        assert_eq!(
            message_path(Path::new("/work/src/main.rs"), cwd),
            "src/main.rs"
        );
        assert_eq!(
            message_path(Path::new("/Users/me/Downloads/report.pdf"), cwd),
            "/Users/me/Downloads/report.pdf"
        );
        // A sibling whose name starts like the folder's is not inside it.
        assert_eq!(
            message_path(Path::new("/workshop/x.rs"), cwd),
            "/workshop/x.rs"
        );
        let files = [file("src/main.rs"), file("notes.txt")];
        assert_eq!(
            with_files("look", &files, cwd),
            "look\n\nAttached files:\n- src/main.rs\n- notes.txt"
        );
        assert_eq!(with_files("look", &[], cwd), "look");
        assert_eq!(
            with_files("", &files, cwd),
            "Attached files:\n- src/main.rs\n- notes.txt"
        );
        assert_eq!(
            with_files(" \n", &files[..1], cwd),
            "Attached files:\n- src/main.rs"
        );
    }

    #[test]
    fn finds_both_token_spellings() {
        let text = "a [Pasted text #1] b [Pasted text#22] [Pasted text #] [Pasted text #3 [Pasted text #4]";
        let found: Vec<_> = find_tokens(text)
            .into_iter()
            .map(|(r, n)| (&text[r], n))
            .collect();
        assert_eq!(
            found,
            vec![
                ("[Pasted text #1]", 1),
                ("[Pasted text#22]", 22),
                ("[Pasted text #4]", 4)
            ]
        );
    }

    /// S7: messages sent before images left the composer keep their
    /// `[Image #N]` text; the conversation still shows them as chips.
    #[test]
    fn history_shows_old_image_tokens_as_chips() {
        let text = chip_text("see [Image #1] and [Image#2], not [Image #] or [Pasted text #1]");
        assert_eq!(text, "see 🖼 #1 and 🖼 #2, not [Image #] or [Pasted text #1]");
        assert_eq!(chip_text("no images"), "no images");
    }

    #[test]
    fn removal_renumbers_later_tokens() {
        let text = "[Pasted text #1] first [Pasted text #2] second [Pasted text#3] third";
        let remove = |text: &str, n| remove_token(text, n);
        assert_eq!(
            remove(text, 2),
            "[Pasted text #1] first second [Pasted text #2] third"
        );
        assert_eq!(
            remove(text, 1),
            "first [Pasted text #1] second [Pasted text #2] third"
        );
        assert_eq!(remove("look [Pasted text #1]", 1), "look");
        assert_eq!(remove("no tokens", 1), "no tokens");

        // Removing #1 then (the new) #1 leaves the third paste as #1.
        let once = remove(text, 1);
        assert_eq!(remove(&once, 1), "first second [Pasted text #1] third");
        // The next paste after removals is numbered by count again.
        let count = find_tokens(&remove(&once, 1)).len();
        assert_eq!(token(count + 1), "[Pasted text #2]");
    }

    /// Images have no composer token any more; every rule the shared token
    /// code has for a `[Pasted text #N]` is the one it always had.
    #[test]
    fn pasted_text_tokens_still_drive_their_pastes() {
        let text = "[Pasted text #1] a [Pasted text#2] b [Pasted text #3]";
        assert_eq!(token(4), "[Pasted text #4]");
        assert!(needs_lead(Some('x')) && !needs_lead(Some('\n')) && !needs_lead(None));
        let numbers: Vec<_> = find_tokens(text)
            .into_iter()
            .map(|(r, n)| (&text[r], n))
            .collect();
        assert_eq!(
            numbers,
            vec![
                ("[Pasted text #1]", 1),
                ("[Pasted text#2]", 2),
                ("[Pasted text #3]", 3)
            ]
        );
        assert_eq!(
            remove_token(text, 2),
            "[Pasted text #1] a b [Pasted text #2]"
        );
        assert_eq!(present("[Pasted text #1] [Pasted text #3]", 3), vec![1, 3]);
        assert_eq!(
            renumber("[Pasted text #1] [Pasted text #3]", &[1, 3]),
            "[Pasted text #1] [Pasted text #2]"
        );
        let pasted = [
            PastedText::new("alpha".into()),
            PastedText::new("beta".into()),
        ];
        assert_eq!(
            expand("B: [Pasted text #2] A:[Pasted text#1]", &pasted),
            "B: beta A:alpha"
        );
    }

    #[test]
    fn deleted_pastes_are_pruned_and_the_rest_renumbered() {
        // #2 of three was deleted from the text; #3 appears twice (copied),
        // #0 and #7 were typed and match no paste.
        let text = "[Pasted text #1] x [Pasted text #3] [Pasted text #0] [Pasted text #3] [Pasted text #7]";
        let kept = present(text, 3);
        assert_eq!(kept, vec![1, 3]);
        assert_eq!(
            renumber(text, &kept),
            "[Pasted text #1] x [Pasted text #2] [Pasted text #0] [Pasted text #2] [Pasted text #7]"
        );
        // The first one deleted: the others all move down.
        let text = "a [Pasted text #2] [Pasted text #3]";
        let kept = present(text, 3);
        assert_eq!(kept, vec![2, 3]);
        assert_eq!(renumber(text, &kept), "a [Pasted text #1] [Pasted text #2]");
        // Nothing deleted: nothing to prune.
        assert_eq!(present("[Pasted text #2] [Pasted text #1]", 2), vec![1, 2]);
        assert!(present("all gone", 2).is_empty());
        assert!(present("[Pasted text #1]", 0).is_empty());
    }

    #[test]
    fn expansion_puts_each_paste_verbatim_in_its_place() {
        let pasted = [
            PastedText::new("alpha\r\n  indented\n".into()),
            PastedText::new("beta [Pasted text #1] [Image #1]".into()),
        ];
        // Out of order, repeated, glued to text and inside other brackets.
        let text = "B: [Pasted text #2] A:[Pasted text#1]! again [Pasted text #1] [Image #1]";
        assert_eq!(
            expand(text, &pasted),
            "B: beta [Pasted text #1] [Image #1] A:alpha\r\n  indented\n! again alpha\r\n  indented\n [Image #1]"
        );
        // Numbers no paste has stay as typed.
        assert_eq!(
            expand("[Pasted text #0] [Pasted text #3]", &pasted),
            "[Pasted text #0] [Pasted text #3]"
        );
        assert_eq!(expand("plain", &[]), "plain");
    }

    #[test]
    fn counts_lines_with_any_line_ending() {
        assert_eq!(line_count(""), 0);
        assert_eq!(line_count("a"), 1);
        assert_eq!(line_count("a\n"), 1);
        assert_eq!(line_count("a\nb"), 2);
        assert_eq!(line_count("a\r\nb\r\n"), 2);
        assert_eq!(line_count("a\r\n\r\nb"), 3);
        assert_eq!(line_count("a\n\n\n"), 3);
    }

    #[test]
    fn long_pastes_start_at_the_thresholds() {
        let lines = |n: usize, end: &str| (1..=n).map(|i| format!("{i}{end}")).collect::<String>();
        assert!(!is_long_paste(&lines(PASTE_LINES - 1, "\n")));
        assert!(is_long_paste(&lines(PASTE_LINES, "\n")));
        assert!(is_long_paste(&lines(PASTE_LINES, "\r\n")));
        // Without the last line break it is still six lines.
        assert!(is_long_paste(lines(PASTE_LINES, "\n").trim_end()));
        // Characters, not bytes: 799 two-byte letters are short.
        assert!(!is_long_paste(&"é".repeat(PASTE_CHARS - 1)));
        assert!(is_long_paste(&"é".repeat(PASTE_CHARS)));
        assert!(!is_long_paste(&"\n".repeat(20)));
        assert!(!is_long_paste(&" ".repeat(PASTE_CHARS * 2)));
    }

    #[test]
    fn pasted_text_keeps_its_text_and_previews_its_start() {
        let text = format!("\tfn main() {{\r\n{}\n}}\n", "x".repeat(60));
        let pasted = PastedText::new(text.clone());
        assert_eq!(pasted.text, text);
        assert_eq!(pasted.lines, 3);
        assert_eq!(
            pasted.preview.as_ref(),
            format!("  fn main() {{\n{}\n}}", "x".repeat(PREVIEW_COLUMNS))
        );
        let long = (0..40).map(|i| format!("{i}\n")).collect::<String>();
        assert_eq!(PastedText::new(long).preview.lines().count(), PREVIEW_LINES);
    }
}
