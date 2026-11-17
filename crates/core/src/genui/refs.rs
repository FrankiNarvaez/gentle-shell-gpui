//! Code references: the optional `ref` an item of any kind carries when it
//! corresponds to code (`{path, line?, endLine?}`, or the shorthand
//! `"path:line"` / `"path:line-end"`). Paths are repo-relative or absolute
//! inside the workspace; a relative one that climbs out of the root is
//! rejected, and an absolute one only resolves inside the root.

use std::path::{Component, Path, PathBuf};

use serde_json::{Map, Value, json};

/// The longest path kept (characters).
pub const MAX_PATH: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeRef {
    /// `/`-separated, without `.` segments or `..` that could be folded.
    pub path: String,
    /// 1-based.
    pub line: Option<u32>,
    /// 1-based, at least `line`; only with a line.
    pub end_line: Option<u32>,
}

impl CodeRef {
    /// A ref from its wire value: an object `{path, line?, endLine?}` or a
    /// string `path`, `path:42`, `path:42-50`. Lines that aren't positive
    /// whole numbers are dropped, an `endLine` before `line` too.
    pub fn parse(value: &Value) -> Result<CodeRef, String> {
        let (path, line, end_line) = match value {
            Value::String(text) => split_lines(text.trim()),
            Value::Object(map) => {
                let Some(path) = map.get("path").and_then(Value::as_str) else {
                    return Err("needs a \"path\"".into());
                };
                (
                    path.trim(),
                    map.get("line").and_then(line_of),
                    map.get("endLine")
                        .or_else(|| map.get("end_line"))
                        .and_then(line_of),
                )
            }
            _ => return Err("must be an object {path, line?, endLine?} or \"path:line\"".into()),
        };
        let path = normalize(path)?;
        let end_line = match (line, end_line) {
            (Some(line), Some(end)) if end > line => Some(end),
            _ => None,
        };
        Ok(CodeRef {
            path,
            line,
            end_line,
        })
    }

    /// `item`'s `ref` field: `None` when absent or null.
    pub fn field(item: &Value) -> Result<Option<CodeRef>, String> {
        match item.get("ref") {
            None | Some(Value::Null) => Ok(None),
            Some(value) => CodeRef::parse(value).map(Some),
        }
    }

    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("path".into(), json!(self.path));
        if let Some(line) = self.line {
            map.insert("line".into(), json!(line));
        }
        if let Some(end) = self.end_line {
            map.insert("endLine".into(), json!(end));
        }
        Value::Object(map)
    }

    /// What a link shows: the file name and the lines, `viewer.rs:42-50`.
    pub fn label(&self) -> String {
        let name = self.path.rsplit('/').next().unwrap_or(&self.path);
        format!("{name}{}", self.lines())
    }

    /// `path:42-50` (the whole path), for text exports and the composer.
    pub fn text(&self) -> String {
        format!("{}{}", self.path, self.lines())
    }

    fn lines(&self) -> String {
        match (self.line, self.end_line) {
            (Some(line), Some(end)) => format!(":{line}-{end}"),
            (Some(line), None) => format!(":{line}"),
            _ => String::new(),
        }
    }

    /// The file on disk under `root`, or `None` when the path is outside it.
    /// Lexical: symlinks are not followed.
    pub fn resolve(&self, root: &Path) -> Option<PathBuf> {
        let root = fold(root);
        let path = Path::new(&self.path);
        let full = if is_absolute(&self.path) {
            fold(path)
        } else {
            fold(&root.join(path))
        };
        full.starts_with(&root).then_some(full)
    }
}

/// `"src/a.rs:42-50"` → path, line, end. A trailing `:n` that isn't a line
/// stays part of the path (Windows drive letters, odd names).
fn split_lines(text: &str) -> (&str, Option<u32>, Option<u32>) {
    let Some((path, tail)) = text.rsplit_once(':') else {
        return (text, None, None);
    };
    let (line, end) = match tail.split_once('-') {
        Some((line, end)) => (line, Some(end)),
        None => (tail, None),
    };
    match line.parse::<u32>() {
        Ok(line) if line > 0 && !path.is_empty() => (
            path,
            Some(line),
            end.and_then(|e| e.parse::<u32>().ok()).filter(|e| *e > 0),
        ),
        _ => (text, None, None),
    }
}

fn line_of(value: &Value) -> Option<u32> {
    let n = value.as_f64()?;
    (n.fract() == 0. && n >= 1. && n <= f64::from(u32::MAX)).then_some(n as u32)
}

fn is_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with('/')
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'/')
}

/// `/` separators, no `.` segments, `..` folded; an error when a relative
/// path climbs out of its root.
fn normalize(path: &str) -> Result<String, String> {
    let path = path.trim().replace('\\', "/");
    let path = path.strip_prefix("file://").unwrap_or(&path);
    if path.is_empty() {
        return Err("has an empty path".into());
    }
    if path.chars().count() > MAX_PATH {
        return Err(format!("path is longer than {MAX_PATH} characters"));
    }
    let absolute = is_absolute(path);
    let (prefix, rest) = if let Some(rest) = path.strip_prefix('/') {
        ("/", rest)
    } else if absolute {
        path.split_at(3)
    } else {
        ("", path)
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() && !absolute {
                    return Err(format!("path \"{path}\" escapes the workspace"));
                }
            }
            part => parts.push(part),
        }
    }
    if parts.is_empty() {
        return Err(format!("path \"{path}\" names no file"));
    }
    Ok(format!("{prefix}{}", parts.join("/")))
}

/// `path` with `.` and `..` folded away, lexically.
fn fold(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(value: Value) -> CodeRef {
        CodeRef::parse(&value).expect("a valid ref")
    }

    #[test]
    fn reads_objects_and_the_string_shorthand() {
        assert_eq!(
            parse(json!({"path": "src/main.rs", "line": 42, "endLine": 50})),
            CodeRef {
                path: "src/main.rs".into(),
                line: Some(42),
                end_line: Some(50)
            }
        );
        assert_eq!(parse(json!("src/a.rs:7")).line, Some(7));
        let range = parse(json!("src/a.rs:7-9"));
        assert_eq!((range.line, range.end_line), (Some(7), Some(9)));
        assert_eq!(parse(json!("src/a.rs")).line, None);
        // Not a line: part of the name.
        assert_eq!(parse(json!("notes:todo")).path, "notes:todo");
    }

    #[test]
    fn drops_lines_that_make_no_sense() {
        let r = parse(json!({"path": "a.rs", "line": 0, "endLine": 3}));
        assert_eq!((r.line, r.end_line), (None, None));
        let r = parse(json!({"path": "a.rs", "line": 9, "endLine": 3}));
        assert_eq!((r.line, r.end_line), (Some(9), None));
        let r = parse(json!({"path": "a.rs", "line": 2.5}));
        assert_eq!(r.line, None);
    }

    #[test]
    fn normalizes_paths() {
        assert_eq!(parse(json!("./src//genui/../lib.rs")).path, "src/lib.rs");
        assert_eq!(parse(json!("src\\win\\a.rs")).path, "src/win/a.rs");
        assert_eq!(parse(json!("/repo/./src/a.rs")).path, "/repo/src/a.rs");
        assert_eq!(parse(json!("C:\\repo\\a.rs")).path, "C:/repo/a.rs");
    }

    #[test]
    fn rejects_paths_escaping_the_workspace() {
        for bad in [
            json!("../secret.txt"),
            json!("src/../../etc/passwd"),
            json!({"path": ".."}),
            json!({"path": "  "}),
            json!({"line": 3}),
            json!(42),
        ] {
            assert!(CodeRef::parse(&bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn field_is_optional() {
        assert_eq!(CodeRef::field(&json!({"id": "a"})), Ok(None));
        assert_eq!(CodeRef::field(&json!({"ref": null})), Ok(None));
        assert!(CodeRef::field(&json!({"ref": "../x"})).is_err());
    }

    #[test]
    fn round_trips_and_labels() {
        let r = parse(json!({"path": "crates/app/src/main.rs", "line": 3, "endLine": 8}));
        assert_eq!(parse(r.to_value()), r);
        assert_eq!(r.label(), "main.rs:3-8");
        assert_eq!(r.text(), "crates/app/src/main.rs:3-8");
        assert_eq!(parse(json!("a.rs")).to_value(), json!({"path": "a.rs"}));
    }

    #[test]
    fn resolves_only_inside_the_root() {
        let root = Path::new("/work/repo");
        assert_eq!(
            parse(json!("src/a.rs")).resolve(root),
            Some(PathBuf::from("/work/repo/src/a.rs"))
        );
        assert_eq!(
            parse(json!("/work/repo/src/a.rs")).resolve(root),
            Some(PathBuf::from("/work/repo/src/a.rs"))
        );
        assert_eq!(parse(json!("/work/other/a.rs")).resolve(root), None);
        assert_eq!(parse(json!("/work/repo/../x.rs")).resolve(root), None);
    }
}
