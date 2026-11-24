//! The user's keymap file on disk: `<data>/gentle-shell-gpui/keymap.json`.
//! The app owns it and only creates it when the user runs "Open keymap file".
//! Parsing and validation live in `gentle_core::keymap`.

use std::io::Write;
use std::path::PathBuf;

use gentle_core::keymap::{self, Keymap};

use super::data_dir;

pub fn path() -> Option<PathBuf> {
    data_dir::app().map(|dir| dir.join("keymap.json"))
}

/// The file's keymap. A missing file is the defaults without an error; an
/// unreadable one is the defaults plus one error.
pub fn load() -> Keymap {
    let Some(path) = path() else {
        return Keymap::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => keymap::parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Keymap::default(),
        Err(e) => {
            let mut k = Keymap::default();
            k.errors.push(keymap::Issue {
                index: None,
                message: format!("couldn't read {}: {e}", path.display()),
            });
            k
        }
    }
}

/// The file's path, created from the template (with `leader`) when missing.
/// An existing file is never touched.
pub fn ensure(leader: &str) -> Result<PathBuf, String> {
    let path = path().ok_or("no data folder on this system")?;
    create_if_missing(&path, leader)?;
    Ok(path)
}

fn create_if_missing(path: &std::path::Path, leader: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("couldn't create {}: {e}", dir.display()))?;
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => file
            .write_all(keymap::template(leader).as_bytes())
            .map_err(|e| format!("couldn't write {}: {e}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(format!("couldn't create {}: {e}", path.display())),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::create_if_missing;

    #[test]
    fn creates_the_template_but_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("gs-keymap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("keymap.json");
        create_if_missing(&file, "ctrl-a").unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("\"leader\": \"ctrl-a\""));
        std::fs::write(&file, "mine").unwrap();
        create_if_missing(&file, "ctrl-a").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "mine");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
