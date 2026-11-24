//! What a prompt carries besides its typed text: images (validation, the RPC
//! payload, and the decode cache conversation thumbnails draw from), regular
//! files sent by path, long pasted texts, and the `[Pasted text #N]` composer
//! tokens that stand for them.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use gpui_kit::{App, Image, ImageFormat, SharedString};
use serde_json::{Value, json};

use gentle_core::transcript::{UserImage, image_key};

/// Largest image accepted, before base64.
pub const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

/// An image waiting in the composer.
#[derive(Clone, Debug)]
pub struct Attachment {
    pub image: Arc<Image>,
    /// The bytes in base64, as sent to Pi.
    pub data: Arc<str>,
    /// [`image_key`] of `data`: the key Pi's echo of this image lands under.
    pub key: u64,
}

impl Attachment {
    /// Accept PNG, JPEG, GIF or WebP bytes (sniffed, not trusted from a
    /// name) up to [`MAX_IMAGE_BYTES`]; BMP and TIFF are converted to PNG
    /// first ([`to_png`]).
    pub fn from_bytes(name: &str, bytes: Vec<u8>) -> Result<Self, Rejection> {
        let bytes = match sniff(&bytes) {
            Some(_) => bytes,
            None => to_png(&bytes).unwrap_or(bytes),
        };
        let size = bytes.len() as u64;
        if size > MAX_IMAGE_BYTES {
            return Err(Rejection::TooLarge {
                name: name.to_string(),
                bytes: size,
            });
        }
        let format = sniff(&bytes).ok_or_else(|| Rejection::Unsupported {
            name: name.to_string(),
        })?;
        let data: Arc<str> = STANDARD.encode(&bytes).into();
        Ok(Self {
            image: Arc::new(Image::from_bytes(format, bytes)),
            key: image_key(&data),
            data,
        })
    }

    pub fn mime_type(&self) -> &'static str {
        self.image.format.mime_type()
    }

    /// Pi's `ImageContent`.
    pub fn payload(&self) -> Value {
        json!({"type": "image", "data": &*self.data, "mimeType": self.mime_type()})
    }
}

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

/// What a dropped or picked path carried.
#[derive(Debug)]
pub enum Loaded {
    Image(Attachment),
    File(AttachedFile),
}

/// Read a path: an image attaches from its bytes ([`Attachment`]: sniffed,
/// capped at [`MAX_IMAGE_BYTES`]); any other regular file attaches by path
/// ([`AttachedFile`]); a folder is refused ([`Rejection::Folder`]).
pub fn load_path(path: &Path) -> Result<Loaded, Rejection> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let unreadable = |error: std::io::Error| Rejection::Unreadable {
        name: name.clone(),
        error: error.to_string(),
    };
    let metadata = std::fs::metadata(path).map_err(unreadable)?;
    if metadata.is_dir() {
        return Err(Rejection::Folder { name });
    }
    if !is_image_file(path) {
        return Ok(Loaded::File(AttachedFile {
            name,
            path: path.to_path_buf(),
            bytes: metadata.len(),
        }));
    }
    if metadata.len() > MAX_IMAGE_BYTES {
        return Err(Rejection::TooLarge {
            name,
            bytes: metadata.len(),
        });
    }
    let bytes = std::fs::read(path).map_err(unreadable)?;
    Attachment::from_bytes(&name, bytes).map(Loaded::Image)
}

/// Whether a path is an image, judged by its first bytes rather than its
/// extension (a misnamed `shot.pnmg` still attaches as an image; a
/// non-image attaches as a file).
pub fn is_image_file(path: &Path) -> bool {
    use std::io::Read as _;
    let mut head = [0u8; 12];
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut read = 0;
    while read < head.len() {
        match file.read(&mut head[read..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => read += n,
        }
    }
    is_image_header(&head[..read])
}

/// The formats [`Attachment::from_bytes`] takes: those [`sniff`] knows, plus
/// BMP (`BM`, then zeroed reserved bytes) and TIFF, which it converts to PNG.
fn is_image_header(head: &[u8]) -> bool {
    sniff(head).is_some()
        || (head.starts_with(b"BM") && head.get(6..10) == Some(&[0; 4]))
        || head.starts_with(b"II*\0")
        || head.starts_with(b"MM\0*")
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

/// A BMP or TIFF re-encoded as PNG, the formats providers don't take but
/// clipboards hand over: Windows' Snipping Tool and Print Screen put a DIB
/// (GPUI gives it as BMP), and some macOS and Linux copies are TIFF. `None`
/// for anything else or bytes that don't decode.
fn to_png(bytes: &[u8]) -> Option<Vec<u8>> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};

    let format = image::guess_format(bytes).ok()?;
    if !matches!(format, image::ImageFormat::Bmp | image::ImageFormat::Tiff) {
        return None;
    }
    let decoded = image::load_from_memory_with_format(bytes, format).ok()?;
    let mut png = Vec::new();
    // Fast: a full-screen capture is encoded while the paste waits on it.
    let encoder =
        PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::Adaptive);
    decoded.write_with_encoder(encoder).ok()?;
    Some(png)
}

/// The image format of `bytes` from its magic number, for the formats Pi's
/// providers accept.
pub fn sniff(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageFormat::Jpeg)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(ImageFormat::Gif)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(ImageFormat::Webp)
    } else {
        None
    }
}

/// RPC `prompt`, steering the running agent when `steer`.
pub fn prompt_command(text: &str, images: &[Attachment], steer: bool) -> Value {
    let mut command = json!({"type": "prompt", "message": text});
    if !images.is_empty() {
        command["images"] = images.iter().map(Attachment::payload).collect();
    }
    if steer {
        command["streamingBehavior"] = "steer".into();
    }
    command
}

// ── Files ──────────────────────────────────────────────────────────────────

/// Where a prompt names `path`: relative to `cwd` with `/` separators when it
/// is inside it, else absolute.
pub fn message_path(path: &Path, cwd: &Path) -> String {
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
pub const PASTE_LINES: usize = 6;
pub const PASTE_CHARS: usize = 800;

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
    pub preview: SharedString,
}

impl PastedText {
    pub fn new(text: String) -> Self {
        Self {
            lines: line_count(&text),
            preview: preview(&text).into(),
            text,
        }
    }
}

/// Lines in `text`: `\n` and `\r\n` both end one, and a final line break
/// does not start another.
pub fn line_count(text: &str) -> usize {
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

// ── Decode cache ───────────────────────────────────────────────────────────

/// Conversation images decoded from base64 once, by [`UserImage::key`].
/// Decoding runs off the UI thread: [`ImageCache::request`] hands out the
/// work and [`ImageCache::finish`] files the results. GPUI keeps its own
/// decoded bitmap of every image drawn until told otherwise, so images
/// this cache forgets are handed back to it ([`ImageCache::release_evicted`]).
#[derive(Default)]
pub struct ImageCache {
    images: HashMap<u64, Slot>,
    /// Forgotten images GPUI may still hold a bitmap of.
    evicted: Vec<Arc<Image>>,
}

enum Slot {
    /// Handed out for decoding; shown as a placeholder meanwhile.
    Decoding,
    /// `None` marks data that is not a displayable image.
    Ready(Option<Arc<Image>>),
}

impl ImageCache {
    /// Seed with a sent attachment so Pi's echo of it needs no decode.
    pub fn insert(&mut self, attachment: &Attachment) {
        self.images
            .insert(attachment.key, Slot::Ready(Some(attachment.image.clone())));
    }

    /// The decoded image; `None` while decoding or when it is not an image.
    pub fn get(&self, image: &UserImage) -> Option<Arc<Image>> {
        match self.images.get(&image.key) {
            Some(Slot::Ready(image)) => image.clone(),
            _ => None,
        }
    }

    /// Mark the shown images not seen yet as decoding and return them to
    /// decode (once per key). With `prune` (`shown` is every image shown),
    /// forget the ones no longer shown.
    pub fn request<'a>(
        &mut self,
        shown: impl IntoIterator<Item = &'a UserImage>,
        prune: bool,
    ) -> Vec<UserImage> {
        let mut keys = HashSet::new();
        let mut jobs = Vec::new();
        for image in shown {
            if prune {
                keys.insert(image.key);
            }
            if let Entry::Vacant(slot) = self.images.entry(image.key) {
                slot.insert(Slot::Decoding);
                jobs.push(image.clone());
            }
        }
        if prune {
            let evicted = &mut self.evicted;
            self.images.retain(|key, slot| {
                let keep = keys.contains(key);
                if let (false, Slot::Ready(Some(image))) = (keep, &*slot) {
                    evicted.push(image.clone());
                }
                keep
            });
        }
        jobs
    }

    /// Drop GPUI's bitmaps of the images forgotten since the last call.
    pub fn release_evicted(&mut self, cx: &mut App) {
        for image in self.evicted.drain(..) {
            image.remove_asset(cx);
        }
    }

    /// Drop GPUI's bitmaps of every image here (the tab is going away).
    pub fn release_all(&mut self, cx: &mut App) {
        self.release_evicted(cx);
        for slot in std::mem::take(&mut self.images).into_values() {
            if let Slot::Ready(Some(image)) = slot {
                image.remove_asset(cx);
            }
        }
    }

    /// File decoded images; ones pruned meanwhile stay forgotten.
    pub fn finish(&mut self, decoded: Vec<(u64, Option<Arc<Image>>)>) {
        for (key, image) in decoded {
            if let Some(slot @ Slot::Decoding) = self.images.get_mut(&key) {
                *slot = Slot::Ready(image);
            }
        }
    }
}

/// Decode `jobs` (from [`ImageCache::request`]); meant for a background thread.
pub fn decode_all(jobs: Vec<UserImage>) -> Vec<(u64, Option<Arc<Image>>)> {
    jobs.iter()
        .map(|image| (image.key, decode(image)))
        .collect()
}

fn decode(image: &UserImage) -> Option<Arc<Image>> {
    let bytes = STANDARD.decode(image.data.as_bytes()).ok()?;
    let format = sniff(&bytes).or_else(|| ImageFormat::from_mime_type(&image.mime_type))?;
    Some(Arc::new(Image::from_bytes(format, bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
    const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10];

    #[test]
    fn sniffs_supported_formats_only() {
        assert_eq!(sniff(PNG), Some(ImageFormat::Png));
        assert_eq!(sniff(JPEG), Some(ImageFormat::Jpeg));
        assert_eq!(sniff(b"GIF89a\x01\0"), Some(ImageFormat::Gif));
        assert_eq!(sniff(b"GIF87a\x01\0"), Some(ImageFormat::Gif));
        assert_eq!(sniff(b"RIFF\x24\0\0\0WEBPVP8 "), Some(ImageFormat::Webp));
        assert_eq!(sniff(b"RIFF\x24\0\0\0WAVEfmt "), None);
        assert_eq!(sniff(b"BM\x36\0\0\0"), None);
        assert_eq!(sniff(b"<svg xmlns"), None);
        assert_eq!(sniff(b""), None);
    }

    /// Windows' Snipping Tool pastes a DIB, which GPUI hands over as BMP;
    /// providers only take PNG, JPEG, GIF and WebP.
    #[test]
    fn bmp_and_tiff_pastes_are_sent_as_png() {
        let pixels =
            image::RgbImage::from_fn(3, 2, |x, y| image::Rgb([x as u8 * 80, y as u8 * 120, 200]));
        for format in [image::ImageFormat::Bmp, image::ImageFormat::Tiff] {
            let mut bytes = std::io::Cursor::new(Vec::new());
            pixels.write_to(&mut bytes, format).unwrap();
            let attachment =
                Attachment::from_bytes("The pasted image", bytes.into_inner()).unwrap();
            assert_eq!(attachment.mime_type(), "image/png", "{format:?}");
            let png = STANDARD.decode(&*attachment.data).unwrap();
            let back = image::load_from_memory_with_format(&png, image::ImageFormat::Png).unwrap();
            assert_eq!(back.to_rgb8(), pixels, "{format:?} keeps its pixels");
        }
        // Something that only starts like a BMP is still refused.
        assert!(matches!(
            Attachment::from_bytes("x", b"BM\x36\0\0\0junk".to_vec()),
            Err(Rejection::Unsupported { .. })
        ));
    }

    #[test]
    fn validates_type_and_size() {
        let ok = Attachment::from_bytes("a.png", PNG.to_vec()).unwrap();
        assert_eq!(ok.mime_type(), "image/png");
        assert_eq!(&*ok.data, STANDARD.encode(PNG));
        assert_eq!(ok.key, image_key(&ok.data));

        // The name lies; the bytes decide.
        assert_eq!(
            Attachment::from_bytes("fake.png", b"hello".to_vec()).unwrap_err(),
            Rejection::Unsupported {
                name: "fake.png".into()
            }
        );

        let mut big = PNG.to_vec();
        big.resize(MAX_IMAGE_BYTES as usize + 1, 0);
        let error = Attachment::from_bytes("big.png", big).unwrap_err();
        assert!(matches!(error, Rejection::TooLarge { bytes, .. } if bytes == MAX_IMAGE_BYTES + 1));
        assert!(error.to_string().contains("20 MB"));

        let mut limit = PNG.to_vec();
        limit.resize(MAX_IMAGE_BYTES as usize, 0);
        assert!(Attachment::from_bytes("limit.png", limit).is_ok());
    }

    /// A folder of this test's own, outside the repository.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gs-attach-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A file as a prompt names it; only its path matters to the block.
    fn file(path: &str) -> AttachedFile {
        AttachedFile {
            name: path.rsplit('/').next().unwrap().to_string(),
            path: PathBuf::from(path),
            bytes: 0,
        }
    }

    /// S1: a file that is not an image attaches by path instead of being
    /// refused for its format; images keep today's behavior.
    #[test]
    fn regular_files_attach_by_path() {
        let dir = temp_dir("files");
        let notes = dir.join("notes.txt");
        std::fs::write(&notes, "hello").unwrap();
        let pdf = dir.join("report.pdf");
        std::fs::write(&pdf, b"%PDF-1.7\n").unwrap();

        let Loaded::File(file) = load_path(&notes).unwrap() else {
            panic!("notes.txt is a file");
        };
        assert_eq!(file.name, "notes.txt");
        assert_eq!(file.path, notes);
        assert_eq!(file.bytes, 5);
        assert_eq!(human_size(file.bytes), "5 B");
        assert!(matches!(load_path(&pdf).unwrap(), Loaded::File(_)));

        let png = dir.join("shot.PNG");
        std::fs::write(&png, PNG).unwrap();
        let Loaded::Image(image) = load_path(&png).unwrap() else {
            panic!("a PNG is still an image");
        };
        assert_eq!(image.mime_type(), "image/png");

        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MB");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// S2: a folder is still refused, by name.
    #[test]
    fn folders_are_rejected_by_name() {
        let dir = temp_dir("folder");
        let folder = dir.join("shots");
        std::fs::create_dir_all(&folder).unwrap();
        let error = load_path(&folder).unwrap_err();
        assert_eq!(
            error,
            Rejection::Folder {
                name: "shots".into()
            }
        );
        assert_eq!(
            error.to_string(),
            "shots is a folder; attach files instead."
        );
        std::fs::remove_dir_all(&dir).unwrap();
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
    fn loads_files_and_reports_failures() {
        let dir = temp_dir("loads");
        let png = dir.join("shot.PNG");
        std::fs::write(&png, PNG).unwrap();
        let text = dir.join("notes.txt");
        std::fs::write(&text, "hi").unwrap();

        let Loaded::Image(loaded) = load_path(&png).unwrap() else {
            panic!("a PNG is an image");
        };
        assert_eq!(loaded.mime_type(), "image/png");
        assert!(matches!(load_path(&text).unwrap(), Loaded::File(_)));
        assert!(matches!(
            load_path(&dir.join("missing.png")),
            Err(Rejection::Unreadable { .. })
        ));
        // Judged by content: a misnamed PNG attaches, a `.png` text doesn't.
        let misnamed = dir.join("bug.pnmg");
        std::fs::write(&misnamed, PNG).unwrap();
        let fake = dir.join("fake.png");
        std::fs::write(&fake, "not an image").unwrap();
        assert!(is_image_file(&png));
        assert!(is_image_file(&misnamed));
        let Loaded::Image(misnamed) = load_path(&misnamed).unwrap() else {
            panic!("a misnamed PNG is still an image");
        };
        assert_eq!(misnamed.mime_type(), "image/png");
        assert!(!is_image_file(&fake));
        assert!(!is_image_file(&text));
        assert!(!is_image_file(&dir.join("missing.png")));
        std::fs::remove_dir_all(&dir).unwrap();
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

    #[test]
    fn serializes_prompt_payload() {
        let image = Attachment::from_bytes("a.png", PNG.to_vec()).unwrap();
        let data = STANDARD.encode(PNG);
        assert_eq!(
            prompt_command("what is this", std::slice::from_ref(&image), false),
            json!({
                "type": "prompt",
                "message": "what is this",
                "images": [{"type": "image", "data": data, "mimeType": "image/png"}],
            })
        );
        assert_eq!(
            prompt_command("again", &[image], true)["streamingBehavior"],
            "steer"
        );
        let plain = prompt_command("hi", &[], false);
        assert_eq!(plain, json!({"type": "prompt", "message": "hi"}));
    }

    #[test]
    fn cache_decodes_once_and_prunes() {
        let attachment = Attachment::from_bytes("a.png", PNG.to_vec()).unwrap();
        let echoed = UserImage::new("image/png", attachment.data.clone());
        let mut cache = ImageCache::default();
        cache.insert(&attachment);
        assert!(Arc::ptr_eq(&cache.get(&echoed).unwrap(), &attachment.image));

        let jpeg = UserImage::new("image/jpeg", STANDARD.encode(JPEG).into());
        let junk = UserImage::new("image/png", "!!not base64".into());
        let jobs = cache.request([&jpeg, &junk, &jpeg], true);
        assert_eq!(jobs.len(), 2);
        // Decoding: a placeholder, and not handed out again.
        assert!(cache.get(&jpeg).is_none());
        assert!(cache.request([&jpeg], false).is_empty());
        // The PNG is no longer shown, so the pruning request dropped it, to
        // be released from GPUI's cache.
        assert!(cache.get(&echoed).is_none());
        assert_eq!(cache.evicted.len(), 1);
        assert!(Arc::ptr_eq(&cache.evicted[0], &attachment.image));

        cache.finish(decode_all(jobs));
        assert_eq!(cache.get(&jpeg).unwrap().format, ImageFormat::Jpeg);
        assert!(cache.get(&junk).is_none());
        assert!(cache.request([&junk], false).is_empty());

        // A conversation with no images forgets them all, though nothing is new.
        assert!(cache.request([], true).is_empty());
        assert!(cache.get(&jpeg).is_none());
        assert_eq!(cache.evicted.len(), 2);
    }

    #[test]
    fn finished_decodes_of_pruned_images_are_dropped() {
        let jpeg = UserImage::new("image/jpeg", STANDARD.encode(JPEG).into());
        let other = UserImage::new("image/png", STANDARD.encode(PNG).into());
        let mut cache = ImageCache::default();
        let jobs = cache.request([&jpeg], true);
        // A new conversation shows only `other` before the decode lands.
        let _ = cache.request([&other], true);
        cache.finish(decode_all(jobs));
        assert!(cache.get(&jpeg).is_none());
        assert_eq!(cache.request([&jpeg], false).len(), 1);
    }
}
