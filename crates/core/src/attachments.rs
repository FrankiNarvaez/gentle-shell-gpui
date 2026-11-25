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

use crate::backend::Backend;

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
    /// The path has a line break in it; the prompt's `Attached files:` block
    /// names one file per line and could not carry it.
    LineBreak {
        name: String,
    },
    /// The path has no UTF-8 spelling, so no line of the prompt could name
    /// the file.
    NotUtf8 {
        name: String,
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
            Self::LineBreak { name } => {
                write!(
                    f,
                    "{name} has a line break in its path; rename it to attach it."
                )
            }
            Self::NotUtf8 { name } => {
                write!(f, "{name} is not valid UTF-8; rename it to attach it.")
            }
        }
    }
}

/// The file's name, as its card and the prompt name it: the last part of
/// `path`, or the whole path when it has none.
pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Why `path` can't be named in a prompt's `Attached files:` block, which
/// names one file per line: a line break in its text, or bytes with no UTF-8
/// spelling. `None` for a path the block can carry.
pub fn unprintable(path: &Path) -> Option<Rejection> {
    let name = file_name(path);
    // Searched as bytes: a non-UTF-8 path has no text to search in.
    let line_break = path
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .any(|byte| matches!(byte, b'\n' | b'\r'));
    if line_break {
        return Some(Rejection::LineBreak { name });
    }
    path.to_str()
        .is_none()
        .then_some(Rejection::NotUtf8 { name })
}

/// Where a prompt names `path`, a path on this computer: as `backend` reads
/// it, relative to `cwd` (read there too) with `/` separators when it is
/// inside it, else absolute. A path the backend can't reach, and a tab with
/// no backend yet, keep this computer's spelling.
fn message_path(path: &Path, cwd: &Path, backend: Option<&Backend>) -> String {
    match backend.and_then(|backend| mapped(path, cwd, backend)) {
        Some((path, cwd)) => relative(Path::new(&path), Path::new(&cwd)),
        None => relative(path, cwd),
    }
}

/// Both ends of [`message_path`] as `backend` reads them, or `None` when it
/// can't reach either.
fn mapped(path: &Path, cwd: &Path, backend: &Backend) -> Option<(String, String)> {
    Some((
        backend.path(&path.to_string_lossy())?,
        backend.path(&cwd.to_string_lossy())?,
    ))
}

/// `path` inside `cwd`: the rest of it with `/` separators; else `path` as it
/// is.
fn relative(path: &Path, cwd: &Path) -> String {
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
pub fn with_files(
    text: &str,
    files: &[AttachedFile],
    cwd: &Path,
    backend: Option<&Backend>,
) -> String {
    if files.is_empty() {
        return text.to_string();
    }
    let block = files
        .iter()
        .map(|file| format!("- {}", message_path(&file.path, cwd, backend)))
        .collect::<Vec<_>>()
        .join("\n");
    if text.trim().is_empty() {
        format!("Attached files:\n{block}")
    } else {
        format!("{text}\n\nAttached files:\n{block}")
    }
}

/// The block [`with_files`] writes: its opening line.
const FILES_HEADER: &str = "Attached files:";

/// Out of a sent message, the text the person wrote and the paths of the
/// `Attached files:` block it carries, in order: only the block
/// [`with_files`] wrote comes out (the message's last lines, a `- ` path per
/// line, after a blank line or at its very start). Anything else — a text
/// that mentions the header, a block that is not last, a line without `- ` —
/// stays in the body. The body has no trailing blank lines.
pub fn split_files(message: &str) -> (&str, Vec<&str>) {
    let trimmed = message.trim_end();
    // Line starts, so the block's lines and the body stay slices of `trimmed`.
    let mut starts = vec![0];
    starts.extend(trimmed.match_indices('\n').map(|(at, _)| at + 1));
    let line = |ix: usize| {
        let start = starts[ix];
        let end = starts.get(ix + 1).copied().unwrap_or(trimmed.len());
        trimmed[start..end]
            .trim_end_matches('\n')
            .trim_end_matches('\r')
    };
    // The block's paths, from the last line up.
    let mut files = Vec::new();
    let mut ix = starts.len();
    while ix > 0 {
        let text = line(ix - 1);
        match text.strip_prefix("- ").filter(|path| !path.is_empty()) {
            Some(path) => files.push(path),
            None => break,
        }
        ix -= 1;
    }
    // Before them, the header; and before it, the blank line the text ended
    // on (nothing at all, when the message is only the block).
    let Some(header) = ix.checked_sub(1).filter(|_| !files.is_empty()) else {
        return (trimmed, Vec::new());
    };
    if line(header) != FILES_HEADER {
        return (trimmed, Vec::new());
    }
    let body = match header.checked_sub(1) {
        None => "",
        Some(blank) if line(blank).is_empty() => trimmed[..starts[blank]].trim_end(),
        Some(_) => return (trimmed, Vec::new()),
    };
    files.reverse();
    (body, files)
}

/// How a sent message reads where its files have no cards (a preview, a
/// title): its body, or the names of the files it attached when it carried
/// no text of its own.
pub fn summary(message: &str) -> String {
    let (body, files) = split_files(message);
    if !body.is_empty() {
        return body.to_string();
    }
    files
        .iter()
        .map(|path| path.rsplit(['/', '\\']).next().unwrap_or(path))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The warning for the `files` `exists` says are gone from disk, naming each
/// on its own line: nothing is sent until they are removed from the
/// composer. `None` when they are all there.
pub fn missing_files(files: &[AttachedFile], exists: impl Fn(&Path) -> bool) -> Option<String> {
    let gone = files
        .iter()
        .filter(|file| !exists(&file.path))
        .map(|file| {
            format!(
                "{} is no longer there; remove it from the attachments to send.",
                file.name
            )
        })
        .collect::<Vec<_>>();
    (!gone.is_empty()).then(|| gone.join("\n"))
}

/// The one warning a set of refused paths gives, naming each on its own
/// line: a single refusal keeps the text it always had. `None` when nothing
/// was refused.
pub fn rejection_warning(rejections: &[Rejection]) -> Option<String> {
    match rejections {
        [] => None,
        [one] => Some(one.to_string()),
        many => Some(
            many.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        ),
    }
}

/// A file's size as its card shows it: "12 B", "2.0 KB", "3.0 MB",
/// "5.0 GB". The tenth shown never reads 1024.0: a size that would round to
/// it shows in the next unit ("1.0 MB", "1.0 GB").
pub fn human_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes < KB {
        return format!("{bytes} B");
    }
    // Tenths of a unit, rounded half up: the digit the card's text shows, so
    // the unit taken is the one that text will name.
    let tenths = |unit: u64| (u128::from(bytes) * 10 + u128::from(unit / 2)) / u128::from(unit);
    let [kilobytes, megabytes, gigabytes] = [KB, MB, GB].map(tenths);
    let (tenths, unit) = if kilobytes < 10240 {
        (kilobytes, "KB")
    } else if megabytes < 10240 {
        (megabytes, "MB")
    } else {
        (gigabytes, "GB")
    };
    format!("{}.{} {unit}", tenths / 10, tenths % 10)
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

    /// A file with a name and a path of its own (a host path, on Windows).
    fn named(name: &str, path: &str) -> AttachedFile {
        AttachedFile {
            name: name.to_string(),
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
            message_path(Path::new("/work/src/main.rs"), cwd, None),
            "src/main.rs"
        );
        assert_eq!(
            message_path(Path::new("/Users/me/Downloads/report.pdf"), cwd, None),
            "/Users/me/Downloads/report.pdf"
        );
        // A sibling whose name starts like the folder's is not inside it.
        assert_eq!(
            message_path(Path::new("/workshop/x.rs"), cwd, None),
            "/workshop/x.rs"
        );
        let files = [file("src/main.rs"), file("notes.txt")];
        assert_eq!(
            with_files("look", &files, cwd, None),
            "look\n\nAttached files:\n- src/main.rs\n- notes.txt"
        );
        assert_eq!(with_files("look", &[], cwd, None), "look");
        assert_eq!(
            with_files("", &files, cwd, None),
            "Attached files:\n- src/main.rs\n- notes.txt"
        );
        assert_eq!(
            with_files(" \n", &files[..1], cwd, None),
            "Attached files:\n- src/main.rs"
        );
    }

    /// S11: a prompt names paths as the agent's backend reads them: both the
    /// file and the folder (WSL reads `C:\a` as `/mnt/c/a`), so a file inside
    /// the folder stays relative there and one outside it is that backend's
    /// path. A path it can't reach, a local tab and a tab with no backend
    /// keep this computer's spelling, as before.
    #[test]
    fn paths_are_named_as_the_agents_backend_reads_them() {
        let wsl = Backend::Wsl {
            distro: "Ubuntu".to_string(),
        };
        let cwd = Path::new(r"C:\proj");
        let files = [
            named("main.rs", r"C:\proj\src\main.rs"),
            named("a.pdf", r"C:\Users\x\Downloads\a.pdf"),
        ];
        assert_eq!(
            with_files("look", &files, cwd, Some(&wsl)),
            "look\n\nAttached files:\n- src/main.rs\n- /mnt/c/Users/x/Downloads/a.pdf"
        );
        // A folder in a WSL share reads the same way.
        assert_eq!(
            with_files(
                "look",
                &[named("a.txt", r"\\wsl.localhost\Ubuntu\home\me\proj\a.txt")],
                Path::new(r"\\wsl.localhost\Ubuntu\home\me\proj"),
                Some(&wsl),
            ),
            "look\n\nAttached files:\n- a.txt"
        );
        // A network path the backend can't reach is named as it is here.
        assert_eq!(
            with_files(
                "look",
                &[named("x.pdf", r"\\nas\share\x.pdf")],
                cwd,
                Some(&wsl),
            ),
            "look\n\nAttached files:\n- \\\\nas\\share\\x.pdf"
        );
        let local = Backend::Local;
        for backend in [None, Some(&local)] {
            assert_eq!(
                with_files("look", &files, cwd, backend),
                "look\n\nAttached files:\n- C:\\proj\\src\\main.rs\n- C:\\Users\\x\\Downloads\\a.pdf"
            );
        }
    }

    /// S9: the block `with_files` writes splits back into the text and the
    /// paths it names, so a sent message shows its files as cards while the
    /// agent still received the block: any text, files with spaces and
    /// unicode, files only, and a message without a block at all.
    #[test]
    fn a_sent_block_splits_back_into_text_and_paths() {
        let cwd = Path::new("/work");
        let files = [
            named(
                "Laboratorio 1.6.2 - Franki Jair.pdf",
                "/Users/me/Documents/Comu 3/Laboratorio 1.6.2 - Franki Jair.pdf",
            ),
            named("notas 日本.txt", "/work/src/notas 日本.txt"),
        ];
        for text in ["Qué te envio?", "línea uno\n\nlínea dos", "  indented  "] {
            let message = with_files(text, &files, cwd, None);
            let (body, paths) = split_files(&message);
            assert_eq!(body, text.trim_end(), "{message:?}");
            assert_eq!(
                paths,
                vec![
                    "/Users/me/Documents/Comu 3/Laboratorio 1.6.2 - Franki Jair.pdf",
                    "src/notas 日本.txt",
                ]
            );
        }
        // A prompt of files only carries no text.
        let only = with_files("", &files, cwd, None);
        assert_eq!(split_files(&only).0, "");
        assert_eq!(split_files(&only).1.len(), 2);
        // One file, and a message that carries no block at all.
        let one = with_files("look", &files[..1], cwd, None);
        assert_eq!(
            split_files(&one),
            (
                "look",
                vec!["/Users/me/Documents/Comu 3/Laboratorio 1.6.2 - Franki Jair.pdf"]
            )
        );
        assert_eq!(split_files("look\n\nmore"), ("look\n\nmore", Vec::new()));
    }

    /// S9: only the trailing block comes out of a message. Text that merely
    /// mentions it, a block that is not last, a list without the header and a
    /// header without the blank line before it stay in the body.
    #[test]
    fn only_a_trailing_block_leaves_the_text() {
        for message in [
            "see Attached files: above",
            "Attached files:\n- a.pdf\nand then more",
            "text\n\nAttached files:\na.pdf",
            "- a.pdf\n- b.txt",
            "text\nAttached files:\n- a.pdf",
            "text\n\nAttached files:\n- a.pdf\nAttached files:\n- b.pdf",
            "text\n\n- a.pdf",
        ] {
            assert_eq!(split_files(message), (message, Vec::new()), "{message:?}");
        }
    }

    /// S9: where its files have no cards, a sent message reads as its body,
    /// or as the names of its files when it carried no text.
    #[test]
    fn a_prompt_reads_as_its_text_or_its_file_names() {
        let cwd = Path::new("/work");
        let files = [
            named("report.pdf", "/work/report.pdf"),
            named("notes.txt", "/work/notes.txt"),
        ];
        assert_eq!(summary(&with_files("look", &files, cwd, None)), "look");
        assert_eq!(
            summary(&with_files("", &files, cwd, None)),
            "report.pdf, notes.txt"
        );
        assert_eq!(summary("plain"), "plain");
        assert_eq!(summary(""), "");
    }

    /// S11: several refused paths give one warning naming each, one per line;
    /// a single refusal keeps the text it always had.
    #[test]
    fn refused_paths_give_one_warning_naming_each() {
        assert_eq!(rejection_warning(&[]), None);
        let folder = Rejection::Folder {
            name: "shots".to_string(),
        };
        assert_eq!(
            rejection_warning(std::slice::from_ref(&folder)).as_deref(),
            Some("shots is a folder; attach files instead.")
        );
        let unsupported = Rejection::Unsupported {
            name: "a.exe".to_string(),
        };
        assert_eq!(
            rejection_warning(&[folder, unsupported]).as_deref(),
            Some(
                "shots is a folder; attach files instead.\n\
                 a.exe is not a PNG, JPEG, GIF, WebP, BMP or TIFF image."
            )
        );
    }

    /// S11: a file gone from disk keeps the prompt from being sent, and the
    /// warning names each one so the user can remove them.
    #[test]
    fn a_gone_file_is_named_and_stops_the_send() {
        let files = [named("a.pdf", "/work/a.pdf"), named("b.txt", "/work/b.txt")];
        assert_eq!(missing_files(&files, |_: &Path| true), None);
        assert_eq!(
            missing_files(&files, |path| !path.ends_with("b.txt")).as_deref(),
            Some("b.txt is no longer there; remove it from the attachments to send.")
        );
        assert_eq!(
            missing_files(&files, |_: &Path| false).as_deref(),
            Some(
                "a.pdf is no longer there; remove it from the attachments to send.\n\
                 b.txt is no longer there; remove it from the attachments to send."
            )
        );
    }

    /// S11: the block names one file per line, so a path with a line break in
    /// it (or one with no UTF-8 spelling) is refused when attached, by name,
    /// instead of breaking the block.
    #[test]
    fn paths_the_block_cannot_carry_are_refused() {
        assert_eq!(unprintable(Path::new("/work/src/main.rs")), None);
        assert_eq!(
            unprintable(Path::new("/work/a\nb.txt")),
            Some(Rejection::LineBreak {
                name: "a\nb.txt".to_string()
            })
        );
        assert_eq!(
            unprintable(Path::new("/work/a\rb.txt")).map(|r| r.to_string()),
            Some("a\rb.txt has a line break in its path; rename it to attach it.".to_string())
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_path_without_a_utf8_spelling_is_refused() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let path = Path::new("/work").join(OsStr::from_bytes(b"bad\xffname.txt"));
        let rejection = unprintable(&path).expect("no UTF-8 spelling");
        assert!(
            matches!(rejection, Rejection::NotUtf8 { .. }),
            "{rejection:?}"
        );
        assert!(rejection.to_string().contains("is not valid UTF-8"));
    }

    /// S11: sizes reach GB, and the tenth a card shows never reads 1024.0: a
    /// size that would round to it shows in the next unit. The small sizes'
    /// text is unchanged.
    #[test]
    fn sizes_reach_gigabytes() {
        assert_eq!(human_size(5), "5 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MB");
        assert_eq!(human_size(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(human_size(5 * 1024 * 1024 * 1024), "5.0 GB");
        // Just below a unit: it rounds up to it instead of reading 1024.0.
        assert_eq!(human_size(1024 * 1024 - 1), "1.0 MB");
        assert_eq!(human_size(1048524), "1023.9 KB");
        assert_eq!(human_size(1048525), "1.0 MB");
        assert_eq!(human_size(1024 * 1024 * 1024 - 1), "1.0 GB");
        assert_eq!(human_size(1073689395), "1023.9 MB");
        assert_eq!(human_size(1073689396), "1.0 GB");
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
