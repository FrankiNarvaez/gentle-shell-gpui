//! Agents using desktop apps, as they use pages with `gentle-browser`: the
//! operations a `gentle-screen` command (or the `screen` tool) asks for —
//! launch an app, list windows, capture one, click, type and press keys in
//! it — how its command line reads, and how a key chord is named. The app
//! runs them through the OS (`features/screen`); this module only reads and
//! words them.
//!
//! Input only reaches apps `gentle-screen launch` started (and their child
//! processes), never an app the user is using: see [`may_drive`].

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// Windows on screen, optionally only an app's (by name) or a process's.
    Windows {
        #[serde(default)]
        app: Option<String>,
        #[serde(default)]
        pid: Option<u32>,
    },
    /// Start `command` and wait (up to `wait` seconds, 20 by default) for
    /// its first window. `app` names the app when the command hands off to
    /// another process (`open -a Calculator`).
    Launch {
        command: Vec<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        app: Option<String>,
        #[serde(default)]
        wait: Option<f64>,
    },
    /// A PNG of a window (by id), of an app's or a process's front window,
    /// or of the whole screen when none is named. Its pixels are the
    /// window's coordinates for `click`.
    Screenshot {
        #[serde(default)]
        window: Option<u64>,
        #[serde(default)]
        app: Option<String>,
        #[serde(default)]
        pid: Option<u32>,
    },
    /// Click at `x`, `y` in the window (its screenshot's pixels).
    Click {
        window: u64,
        x: f64,
        y: f64,
        #[serde(default)]
        button: Button,
        #[serde(default = "one")]
        count: u8,
    },
    /// Type `text` into the window's focused field.
    Type { window: u64, text: String },
    /// Press keys: `enter`, `cmd+s`, `ctrl+shift+tab`; several, spaced.
    Key { window: u64, keys: String },
    /// Scroll by `dy` (positive is down) at `x`, `y`, or the window's middle.
    Scroll {
        window: u64,
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
        dy: f64,
    },
    /// Stop an app `launch` started.
    Quit { pid: u32 },
}

fn one() -> u8 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Button {
    #[default]
    Left,
    Right,
    Middle,
}

impl Op {
    /// The window an input op drives.
    pub fn input_window(&self) -> Option<u64> {
        match self {
            Op::Click { window, .. }
            | Op::Type { window, .. }
            | Op::Key { window, .. }
            | Op::Scroll { window, .. } => Some(*window),
            _ => None,
        }
    }
}

/// One window, as `windows` lists it. Coordinates are the OS's for input
/// (points on macOS, pixels on Windows and Linux), top-left origin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Window {
    pub id: u64,
    pub pid: u32,
    pub app: String,
    pub title: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// The answer the command prints: `{"ok": …}` or `{"error": {kind, message}}`.
pub fn ok(value: Value) -> Value {
    json!({ "ok": value })
}

pub fn error(kind: &str, message: impl Into<String>) -> Value {
    json!({ "error": { "kind": kind, "message": message.into() } })
}

/// Whether input may reach a window of `pid`: it is a process `launch`
/// started, or one of their children (`parent` names a process's parent).
/// Everything else is the user's.
pub fn may_drive(pid: u32, launched: &[u32], parent: impl Fn(u32) -> Option<u32>) -> bool {
    let mut current = Some(pid);
    // A process tree is shallow; the bound only guards a cycle.
    for _ in 0..64 {
        let Some(p) = current else {
            return false;
        };
        if launched.contains(&p) {
            return true;
        }
        if p <= 1 {
            return false;
        }
        current = parent(p).filter(|parent| *parent != p);
    }
    false
}

/// The window `windows` names by app (case-insensitive substring of the
/// app's name) or pid: the first one listed, which is the frontmost.
pub fn pick<'a>(windows: &'a [Window], app: Option<&str>, pid: Option<u32>) -> Option<&'a Window> {
    windows.iter().find(|w| matches(w, app, pid))
}

/// `w` belongs to `app` (case-insensitive substring) and `pid`, when given.
pub fn matches(w: &Window, app: Option<&str>, pid: Option<u32>) -> bool {
    app.is_none_or(|app| w.app.to_lowercase().contains(&app.to_lowercase()))
        && pid.is_none_or(|pid| w.pid == pid)
}

// ── Keys ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    /// ⌘ on macOS; the Windows key elsewhere.
    pub cmd: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// A character key, lowercase (`a`, `1`, `/`).
    Char(char),
    Enter,
    Tab,
    Escape,
    Backspace,
    Delete,
    Space,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    /// F1–F12.
    F(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    pub modifiers: Modifiers,
    pub key: Key,
}

/// `cmd+shift+s`, `Enter`, `ctrl+tab`: modifiers then one key, joined by
/// `+`. `meta`/`super`/`win` read as cmd, `option`/`opt` as alt, `control`
/// as ctrl.
pub fn parse_chord(text: &str) -> Result<Chord, String> {
    let bad = || format!("not a key: {text}");
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    let (key, mods) = parts.split_last().ok_or_else(bad)?;
    let mut modifiers = Modifiers::default();
    for m in mods {
        match m.to_lowercase().as_str() {
            "cmd" | "command" | "meta" | "super" | "win" => modifiers.cmd = true,
            "ctrl" | "control" => modifiers.ctrl = true,
            "alt" | "option" | "opt" => modifiers.alt = true,
            "shift" => modifiers.shift = true,
            _ => return Err(bad()),
        }
    }
    let lower = key.to_lowercase();
    let key = match lower.as_str() {
        "enter" | "return" => Key::Enter,
        "tab" => Key::Tab,
        "esc" | "escape" => Key::Escape,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "space" => Key::Space,
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        f if f.len() >= 2
            && f.starts_with('f')
            && f[1..].parse::<u8>().is_ok_and(|n| (1..=12).contains(&n)) =>
        {
            Key::F(f[1..].parse().map_err(|_| bad())?)
        }
        _ => {
            let mut chars = lower.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if !c.is_whitespace() => Key::Char(c),
                _ => return Err(bad()),
            }
        }
    };
    Ok(Chord { modifiers, key })
}

/// Every chord in `keys` (space-separated).
pub fn parse_keys(keys: &str) -> Result<Vec<Chord>, String> {
    let chords: Result<Vec<Chord>, String> = keys.split_whitespace().map(parse_chord).collect();
    match chords {
        Ok(c) if c.is_empty() => Err("no keys given".into()),
        other => other,
    }
}

// ── The command line ────────────────────────────────────────────────────────

pub const USAGE: &str = "\
gentle-screen: launch desktop apps, look at their windows and use them

  gentle-screen launch [--app NAME] [--wait S] [--cwd DIR] -- <command…>
  gentle-screen windows [--app NAME] [--pid N]
  gentle-screen screenshot [--window ID | --app NAME | --pid N]   (the whole screen without one)
  gentle-screen click <window> <x> <y> [--right] [--double]
  gentle-screen type <window> <text>
  gentle-screen key <window> <keys>        enter, cmd+s, ctrl+shift+tab (several, spaced)
  gentle-screen scroll <window> <dy> [--at X,Y]
  gentle-screen quit <pid>
  gentle-screen --json '<op>'

x and y are pixels of that window's screenshot. Input only reaches apps
`launch` started. Answers {\"ok\": …} or {\"error\": {\"kind\", \"message\"}} (exit 1).";

/// The op in `gentle-screen`'s arguments.
pub fn parse_args(args: &[String]) -> Result<Op, String> {
    // `launch … -- command`: everything after `--` is the command.
    let (args, command) = match args.iter().position(|a| a == "--") {
        Some(at) => (&args[..at], args[at + 1..].to_vec()),
        None => (args, Vec::new()),
    };
    let mut words: Vec<&str> = Vec::new();
    let mut flags: Vec<(&str, Option<&str>)> = Vec::new();
    let valued = [
        "--app", "--pid", "--window", "--wait", "--cwd", "--at", "--json",
    ];
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if valued.contains(&arg) {
            flags.push((arg, args.get(i + 1).map(String::as_str)));
            i += 2;
        } else if arg.starts_with("--") && arg.len() > 2 {
            flags.push((arg, None));
            i += 1;
        } else {
            words.push(arg);
            i += 1;
        }
    }
    let flag = |name: &str| flags.iter().find(|(f, _)| *f == name).and_then(|(_, v)| *v);
    let has = |name: &str| flags.iter().any(|(f, _)| *f == name);
    if let Some(json) = flag("--json") {
        return serde_json::from_str(json).map_err(|e| format!("--json: {e}"));
    }
    let usage = || USAGE.to_string();
    let word = |n: usize| words.get(n).copied().ok_or_else(usage);
    let number = |text: &str, what: &str| -> Result<f64, String> {
        text.parse::<f64>()
            .map_err(|_| format!("{what} must be a number"))
    };
    let window = || -> Result<u64, String> {
        word(1)?
            .parse()
            .map_err(|_| "the window is its id, from `gentle-screen windows`".to_string())
    };
    let pid = |text: Option<&str>| -> Result<Option<u32>, String> {
        text.map(|p| {
            p.parse()
                .map_err(|_| "--pid takes a process id".to_string())
        })
        .transpose()
    };
    match word(0)? {
        "windows" => Ok(Op::Windows {
            app: flag("--app").map(str::to_string),
            pid: pid(flag("--pid"))?,
        }),
        "launch" => {
            if command.is_empty() {
                return Err("launch needs a command after `--`".into());
            }
            Ok(Op::Launch {
                command,
                cwd: flag("--cwd").map(str::to_string),
                app: flag("--app").map(str::to_string),
                wait: flag("--wait").map(|w| number(w, "--wait")).transpose()?,
            })
        }
        "screenshot" => Ok(Op::Screenshot {
            window: flag("--window")
                .map(|w| {
                    w.parse()
                        .map_err(|_| "--window takes a window id".to_string())
                })
                .transpose()?,
            app: flag("--app").map(str::to_string),
            pid: pid(flag("--pid"))?,
        }),
        "click" => Ok(Op::Click {
            window: window()?,
            x: number(word(2)?, "x")?,
            y: number(word(3)?, "y")?,
            button: if has("--right") {
                Button::Right
            } else if has("--middle") {
                Button::Middle
            } else {
                Button::Left
            },
            count: if has("--double") { 2 } else { 1 },
        }),
        "type" => Ok(Op::Type {
            window: window()?,
            text: words
                .get(2..)
                .filter(|w| !w.is_empty())
                .ok_or_else(usage)?
                .join(" "),
        }),
        "key" => {
            let keys = words
                .get(2..)
                .filter(|w| !w.is_empty())
                .ok_or_else(usage)?
                .join(" ");
            parse_keys(&keys)?;
            Ok(Op::Key {
                window: window()?,
                keys,
            })
        }
        "scroll" => {
            let at = flag("--at")
                .map(|at| {
                    let (x, y) = at.split_once(',').ok_or("--at takes X,Y")?;
                    Ok::<_, String>((number(x.trim(), "x")?, number(y.trim(), "y")?))
                })
                .transpose()?;
            Ok(Op::Scroll {
                window: window()?,
                dy: number(word(2)?, "dy")?,
                x: at.map(|a| a.0),
                y: at.map(|a| a.1),
            })
        }
        "quit" => Ok(Op::Quit {
            pid: word(1)?
                .parse()
                .map_err(|_| "quit takes a process id".to_string())?,
        }),
        _ => Err(usage()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split(' ').map(str::to_string).collect()
    }

    #[test]
    fn the_command_line_reads_each_op() {
        assert_eq!(
            parse_args(&args(
                "launch --app Calculator --wait 5 -- open -a Calculator"
            ))
            .unwrap(),
            Op::Launch {
                command: vec!["open".into(), "-a".into(), "Calculator".into()],
                cwd: None,
                app: Some("Calculator".into()),
                wait: Some(5.),
            }
        );
        assert!(parse_args(&args("launch --app X")).is_err());
        assert_eq!(
            parse_args(&args("click 42 10 20.5 --double")).unwrap(),
            Op::Click {
                window: 42,
                x: 10.,
                y: 20.5,
                button: Button::Left,
                count: 2
            }
        );
        assert_eq!(
            parse_args(&args("type 42 hello world")).unwrap(),
            Op::Type {
                window: 42,
                text: "hello world".into()
            }
        );
        assert!(parse_args(&args("key 42 cmd+nope+s")).is_err());
        assert_eq!(
            parse_args(&args("scroll 42 300 --at 5,6")).unwrap(),
            Op::Scroll {
                window: 42,
                x: Some(5.),
                y: Some(6.),
                dy: 300.
            }
        );
        assert_eq!(
            parse_args(&args("screenshot --app Safari")).unwrap(),
            Op::Screenshot {
                window: None,
                app: Some("Safari".into()),
                pid: None
            }
        );
        assert!(parse_args(&args("click notanid 1 2")).is_err());
        let json = r#"{"op":"click","window":7,"x":1,"y":2}"#;
        assert_eq!(
            serde_json::from_str::<Op>(json).unwrap(),
            Op::Click {
                window: 7,
                x: 1.,
                y: 2.,
                button: Button::Left,
                count: 1
            }
        );
    }

    #[test]
    fn chords_read_modifiers_then_a_key() {
        let chord = parse_chord("Cmd+Shift+S").unwrap();
        assert!(chord.modifiers.cmd && chord.modifiers.shift && !chord.modifiers.ctrl);
        assert_eq!(chord.key, Key::Char('s'));
        assert_eq!(parse_chord("enter").unwrap().key, Key::Enter);
        assert_eq!(parse_chord("ctrl+F5").unwrap().key, Key::F(5));
        assert_eq!(parse_chord("/").unwrap().key, Key::Char('/'));
        assert!(parse_chord("hyper+a").is_err());
        assert!(parse_chord("abc").is_err());
        assert_eq!(parse_keys("tab tab enter").unwrap().len(), 3);
        assert!(parse_keys("  ").is_err());
    }

    #[test]
    fn input_reaches_launched_apps_and_their_children_only() {
        // 100 launched 200, which started 300; 500 is the user's.
        let parent = |pid: u32| match pid {
            300 => Some(200),
            200 => Some(100),
            100 | 500 => Some(1),
            _ => None,
        };
        assert!(may_drive(100, &[100], parent));
        assert!(may_drive(300, &[100], parent));
        assert!(!may_drive(500, &[100], parent));
        assert!(!may_drive(500, &[], parent));
        // A loop in a broken process table ends.
        assert!(!may_drive(7, &[100], |p| Some(p + 1)));
    }

    #[test]
    fn windows_are_picked_by_app_or_pid() {
        let w = |id, pid, app: &str| Window {
            id,
            pid,
            app: app.into(),
            title: String::new(),
            x: 0.,
            y: 0.,
            width: 1.,
            height: 1.,
        };
        let list = [
            w(1, 10, "Finder"),
            w(2, 20, "Calculator"),
            w(3, 20, "Calculator"),
        ];
        assert_eq!(pick(&list, Some("calc"), None).map(|w| w.id), Some(2));
        assert_eq!(pick(&list, None, Some(10)).map(|w| w.id), Some(1));
        assert_eq!(pick(&list, Some("finder"), Some(20)), None);
    }
}
