//! `gentle-screen` on Linux: X11 (or XWayland) through x11rb. Windows are
//! the window manager's client list, captures are the root window's pixels
//! under a window (what a compositor shows), and input is XTEST.

use std::io::Cursor;
use std::time::Duration;

use gentle_core::screen::{Button, Chord, Key, Window};
use x11rb::connection::{Connection as _, RequestConnection as _};
use x11rb::protocol::xproto::{
    self, Atom, AtomEnum, ClientMessageEvent, ConfigureWindowAux, ConnectionExt as _, EventMask,
    ImageFormat, ImageOrder, MapState, StackMode,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

/// An X connection and its first screen's root window.
struct Display {
    conn: RustConnection,
    root: xproto::Window,
    width: u16,
    height: u16,
}

fn x(e: impl std::fmt::Display) -> String {
    format!("X11: {e}")
}

fn connect() -> Result<Display, String> {
    if std::env::var_os("DISPLAY").is_none_or(|d| d.is_empty()) {
        return Err("gentle-screen needs an X11 session (or XWayland)".into());
    }
    let (conn, screen) = x11rb::connect(None).map_err(x)?;
    let s = &conn.setup().roots[screen];
    let (root, width, height) = (s.root, s.width_in_pixels, s.height_in_pixels);
    Ok(Display {
        conn,
        root,
        width,
        height,
    })
}

impl Display {
    fn atom(&self, name: &str) -> Result<Atom, String> {
        Ok(self
            .conn
            .intern_atom(false, name.as_bytes())
            .map_err(x)?
            .reply()
            .map_err(x)?
            .atom)
    }

    /// A property's raw value, or `None` when the window has none.
    fn property(
        &self,
        window: xproto::Window,
        name: impl Into<Atom>,
        kind: impl Into<Atom>,
    ) -> Option<xproto::GetPropertyReply> {
        let reply = self
            .conn
            .get_property(false, window, name, kind, 0, u32::MAX / 4)
            .ok()?
            .reply()
            .ok()?;
        (reply.format != 0).then_some(reply)
    }

    /// Wait until the server has handled every request sent.
    fn sync(&self) -> Result<(), String> {
        self.conn.get_input_focus().map_err(x)?.reply().map_err(x)?;
        Ok(())
    }

    fn fake(&self, kind: u8, detail: u8, x_: i16, y: i16) -> Result<(), String> {
        self.conn
            .xtest_fake_input(kind, detail, 0, self.root, x_, y, 0)
            .map_err(x)?;
        Ok(())
    }

    /// A key (keycode) down or up.
    fn press(&self, code: u8, down: bool) -> Result<(), String> {
        let kind = if down {
            xproto::KEY_PRESS_EVENT
        } else {
            xproto::KEY_RELEASE_EVENT
        };
        self.fake(kind, code, 0, 0)
    }

    /// Move the pointer to `x`, `y` of `w`.
    fn move_to(&self, w: &Window, x_: f64, y: f64) -> Result<(), String> {
        let (x_, y) = ((w.x + x_).round() as i16, (w.y + y).round() as i16);
        self.fake(xproto::MOTION_NOTIFY_EVENT, 0, x_, y)
    }

    fn button(&self, button: u8) -> Result<(), String> {
        self.fake(xproto::BUTTON_PRESS_EVENT, button, 0, 0)?;
        self.fake(xproto::BUTTON_RELEASE_EVENT, button, 0, 0)
    }

    /// A PNG of the root window's `x`, `y`, `width`×`height` (clamped to the screen).
    fn capture(&self, x_: f64, y: f64, width: f64, height: f64) -> Result<Vec<u8>, String> {
        let left = x_.max(0.).round() as i32;
        let top = y.max(0.).round() as i32;
        let right = ((x_ + width).round() as i32).min(i32::from(self.width));
        let bottom = ((y + height).round() as i32).min(i32::from(self.height));
        if right <= left || bottom <= top {
            return Err("the window is off screen".into());
        }
        let (w, h) = ((right - left) as u16, (bottom - top) as u16);
        let image = self
            .conn
            .get_image(
                ImageFormat::Z_PIXMAP,
                self.root,
                left as i16,
                top as i16,
                w,
                h,
                !0,
            )
            .map_err(x)?
            .reply()
            .map_err(x)?;
        let setup = self.conn.setup();
        let bits = setup
            .pixmap_formats
            .iter()
            .find(|f| f.depth == image.depth)
            .map(|f| f.bits_per_pixel);
        if !matches!(image.depth, 24 | 32)
            || bits != Some(32)
            || setup.image_byte_order != ImageOrder::LSB_FIRST
        {
            return Err(format!(
                "unsupported X11 screen format (depth {})",
                image.depth
            ));
        }
        let (w, h) = (u32::from(w), u32::from(h));
        // Rows are 32-bit pixels, BGRX in memory.
        let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
        for [b, g, r, _] in image
            .data
            .as_chunks::<4>()
            .0
            .iter()
            .take(w as usize * h as usize)
        {
            rgba.extend_from_slice(&[*r, *g, *b, 255]);
        }
        let buffer = image::RgbaImage::from_raw(w, h, rgba)
            .ok_or("the X11 image is shorter than its size")?;
        let mut png = Cursor::new(Vec::new());
        buffer
            .write_to(&mut png, image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        Ok(png.into_inner())
    }
}

/// Visible app windows, frontmost first.
pub fn windows() -> Result<Vec<Window>, String> {
    let d = connect()?;
    let stacking = d.atom("_NET_CLIENT_LIST_STACKING")?;
    let list = d.atom("_NET_CLIENT_LIST")?;
    let mut ids: Vec<u32> = d
        .property(d.root, stacking, AtomEnum::WINDOW)
        .or_else(|| d.property(d.root, list, AtomEnum::WINDOW))
        .and_then(|p| p.value32().map(Iterator::collect))
        .ok_or("no window list: gentle-screen needs a window manager")?;
    // Stacking lists bottom to top.
    ids.reverse();
    let kind = d.atom("_NET_WM_WINDOW_TYPE")?;
    let normal = d.atom("_NET_WM_WINDOW_TYPE_NORMAL")?;
    let dialog = d.atom("_NET_WM_WINDOW_TYPE_DIALOG")?;
    let pid = d.atom("_NET_WM_PID")?;
    let name = d.atom("_NET_WM_NAME")?;
    let utf8 = d.atom("UTF8_STRING")?;
    let mut found = Vec::new();
    for id in ids {
        let Ok(attributes) = d.conn.get_window_attributes(id).map_err(x)?.reply() else {
            continue;
        };
        if attributes.map_state != MapState::VIEWABLE {
            continue;
        }
        let kinds: Option<Vec<u32>> = d
            .property(id, kind, AtomEnum::ATOM)
            .and_then(|p| p.value32().map(Iterator::collect));
        if kinds.is_some_and(|k| !k.contains(&normal) && !k.contains(&dialog)) {
            continue;
        }
        let (Ok(geometry), Ok(origin)) = (
            d.conn.get_geometry(id).map_err(x)?.reply(),
            d.conn
                .translate_coordinates(id, d.root, 0, 0)
                .map_err(x)?
                .reply(),
        ) else {
            continue;
        };
        let text = |p: xproto::GetPropertyReply| String::from_utf8_lossy(&p.value).into_owned();
        let title = d
            .property(id, name, utf8)
            .or_else(|| d.property(id, AtomEnum::WM_NAME, AtomEnum::ANY))
            .map(text)
            .unwrap_or_default();
        // WM_CLASS is "instance\0class\0".
        let app = d
            .property(id, AtomEnum::WM_CLASS, AtomEnum::STRING)
            .map(|p| {
                let parts: Vec<&[u8]> = p.value.split(|b| *b == 0).collect();
                String::from_utf8_lossy(parts.get(1).filter(|c| !c.is_empty()).unwrap_or(&parts[0]))
                    .into_owned()
            })
            .unwrap_or_default();
        found.push(Window {
            id: u64::from(id),
            pid: d
                .property(id, pid, AtomEnum::CARDINAL)
                .and_then(|p| p.value32()?.next())
                .unwrap_or(0),
            app,
            title,
            x: f64::from(origin.dst_x),
            y: f64::from(origin.dst_y),
            width: f64::from(geometry.width),
            height: f64::from(geometry.height),
        });
    }
    Ok(found)
}

/// A PNG of what the screen shows where `w` is.
pub fn capture_window(w: &Window) -> Result<Vec<u8>, String> {
    connect()?.capture(w.x, w.y, w.width, w.height)
}

/// A PNG of the whole screen.
pub fn capture_screen() -> Result<Vec<u8>, String> {
    let d = connect()?;
    d.capture(0., 0., f64::from(d.width), f64::from(d.height))
}

/// A process's parent, from `/proc/<pid>/stat`.
pub fn parent(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // "pid (comm) state ppid …"; comm may hold spaces and parentheses.
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(1)?.parse().ok()
}

pub fn alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

/// Ok when the X server takes fake input (XTEST).
pub fn input_ready() -> Result<(), String> {
    let d = connect()?;
    let xtest = d
        .conn
        .extension_information(x11rb::protocol::xtest::X11_EXTENSION_NAME)
        .map_err(x)?;
    match xtest {
        Some(_) => Ok(()),
        None => Err("the X server has no XTEST extension, so input can't be sent".into()),
    }
}

/// Ask the window manager to activate `w`, and raise it.
pub fn focus(w: &Window) -> Result<(), String> {
    let d = connect()?;
    let id = w.id as u32;
    let active = d.atom("_NET_ACTIVE_WINDOW")?;
    // Source 2: a pager or tool acting for the user, which WMs obey.
    let event = ClientMessageEvent::new(32, id, active, [2, 0, 0, 0, 0]);
    d.conn
        .send_event(
            false,
            d.root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            event,
        )
        .map_err(x)?;
    d.conn
        .configure_window(id, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))
        .map_err(x)?;
    d.sync()?;
    std::thread::sleep(Duration::from_millis(100));
    Ok(())
}

pub fn click(w: &Window, x_: f64, y: f64, button: Button, count: u8) -> Result<(), String> {
    let d = connect()?;
    d.move_to(w, x_, y)?;
    let button = match button {
        Button::Left => 1,
        Button::Middle => 2,
        Button::Right => 3,
    };
    for _ in 0..count.max(1) {
        d.button(button)?;
    }
    d.sync()
}

/// Scroll by wheel notches (buttons 4 up, 5 down), one per ~100 px.
pub fn scroll(w: &Window, x_: f64, y: f64, dy: f64) -> Result<(), String> {
    let d = connect()?;
    d.move_to(w, x_, y)?;
    let button = if dy < 0. { 4 } else { 5 };
    let notches = (dy.abs() / 100.).round().max(1.) as u32;
    for _ in 0..notches {
        d.button(button)?;
    }
    d.sync()
}

pub fn type_text(_: &Window, text: &str) -> Result<(), String> {
    let d = connect()?;
    let keyboard = Keyboard::load(&d)?;
    for c in text.chars() {
        let sym = match c {
            '\n' | '\r' => 0xff0d,
            '\t' => 0xff09,
            c => char_keysym(c),
        };
        keyboard.tap(&d, sym, false)?;
    }
    d.sync()
}

/// Press `chord`'s modifiers and key, then release them in reverse.
pub fn key(_: &Window, chord: &Chord) -> Result<(), String> {
    let d = connect()?;
    let keyboard = Keyboard::load(&d)?;
    let m = chord.modifiers;
    let mut held = Vec::new();
    for (on, sym, name) in [
        (m.cmd, SUPER_L, "Super"),
        (m.ctrl, 0xffe3, "Control"),
        (m.alt, 0xffe9, "Alt"),
        (m.shift, SHIFT_L, "Shift"),
    ] {
        if on {
            let (code, _) = keyboard
                .find(sym)
                .ok_or_else(|| format!("the keyboard has no {name} key"))?;
            d.press(code, true)?;
            held.push(code);
        }
    }
    let sym = match chord.key {
        Key::Char(c) => char_keysym(c),
        Key::Enter => 0xff0d,
        Key::Tab => 0xff09,
        Key::Escape => 0xff1b,
        Key::Backspace => 0xff08,
        Key::Delete => 0xffff,
        Key::Space => 0x20,
        Key::Left => 0xff51,
        Key::Up => 0xff52,
        Key::Right => 0xff53,
        Key::Down => 0xff54,
        Key::Home => 0xff50,
        Key::End => 0xff57,
        Key::PageUp => 0xff55,
        Key::PageDown => 0xff56,
        Key::F(n) => 0xffbe + u32::from(n.max(1)) - 1,
    };
    let tapped = keyboard.tap(&d, sym, m.shift);
    for code in held.into_iter().rev() {
        d.press(code, false)?;
    }
    tapped?;
    d.sync()
}

const SHIFT_L: u32 = 0xffe1;
const SUPER_L: u32 = 0xffeb;

/// A character's keysym: Latin-1 as is, the rest as Unicode keysyms.
fn char_keysym(c: char) -> u32 {
    let cp = u32::from(c);
    if (0x20..=0x7e).contains(&cp) || (0xa0..=0xff).contains(&cp) {
        cp
    } else {
        0x0100_0000 + cp
    }
}

/// The server's keycode → keysyms table.
struct Keyboard {
    first: u8,
    per: usize,
    syms: Vec<u32>,
}

impl Keyboard {
    fn load(d: &Display) -> Result<Self, String> {
        let setup = d.conn.setup();
        let (first, last) = (setup.min_keycode, setup.max_keycode);
        let reply = d
            .conn
            .get_keyboard_mapping(first, last - first + 1)
            .map_err(x)?
            .reply()
            .map_err(x)?;
        Ok(Self {
            first,
            per: usize::from(reply.keysyms_per_keycode),
            syms: reply.keysyms,
        })
    }

    /// The keycode typing `sym`, and whether it needs Shift.
    fn find(&self, sym: u32) -> Option<(u8, bool)> {
        let per = self.per.max(1);
        (0..2).find_map(|level| {
            self.syms
                .chunks(per)
                .position(|row| row.get(level) == Some(&sym))
                .map(|i| (self.first + i as u8, level == 1))
        })
    }

    /// A keycode with no keysyms, free to borrow.
    fn spare(&self) -> Option<u8> {
        self.syms
            .chunks(self.per.max(1))
            .rposition(|row| row.iter().all(|s| *s == 0))
            .map(|i| self.first + i as u8)
    }

    /// Press and release the key typing `sym` (with Shift when its level
    /// needs it and `shifted` doesn't hold it already). A keysym the
    /// keyboard lacks goes on a spare keycode for the press.
    fn tap(&self, d: &Display, sym: u32, shifted: bool) -> Result<(), String> {
        if let Some((code, shift)) = self.find(sym) {
            let shift = match self.find(SHIFT_L) {
                Some((s, _)) if shift && !shifted => Some(s),
                _ => None,
            };
            if let Some(s) = shift {
                d.press(s, true)?;
            }
            d.press(code, true)?;
            d.press(code, false)?;
            if let Some(s) = shift {
                d.press(s, false)?;
            }
            return Ok(());
        }
        let code = self
            .spare()
            .ok_or_else(|| format!("no key types U+{sym:04X} and no keycode is free"))?;
        let per = self.per.max(1) as u8;
        d.conn
            .change_keyboard_mapping(1, code, per, &vec![sym; usize::from(per)])
            .map_err(x)?;
        d.sync()?;
        d.press(code, true)?;
        d.press(code, false)?;
        d.sync()?;
        // Apps read the new mapping lazily; restore it once they have.
        std::thread::sleep(Duration::from_millis(50));
        d.conn
            .change_keyboard_mapping(1, code, per, &vec![0; usize::from(per)])
            .map_err(x)?;
        d.sync()
    }
}
