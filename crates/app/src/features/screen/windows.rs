//! `gentle-screen` on Windows: windows through `EnumWindows` and DWM,
//! pictures through `PrintWindow` and GDI, input through `SendInput`. The
//! process is per-monitor DPI aware, so every coordinate is physical pixels.

use std::io::Cursor;
use std::path::Path;
use std::sync::Once;
use std::time::Duration;

use gentle_core::screen::{Button, Chord, Key, Window};
use image::RgbaImage;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT, STILL_ACTIVE};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiFlush, GetDC, HDC, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, GetExitCodeProcess, OpenProcess, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, MOUSE_EVENT_FLAGS,
    MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
    MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEINPUT, MapVirtualKeyW, SendInput, VIRTUAL_KEY,
    VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_HOME, VK_LEFT, VK_LWIN,
    VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP, VkKeyScanW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GWL_EXSTYLE, GetForegroundWindow, GetShellWindow,
    GetSystemMetrics, GetWindowLongW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, PW_RENDERFULLCONTENT, SM_CXSCREEN,
    SM_CXVIRTUALSCREEN, SM_CYSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SW_RESTORE, SetForegroundWindow, ShowWindow, WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, PWSTR};

/// Visible, titled app windows, frontmost first.
pub fn windows() -> Result<Vec<Window>, String> {
    dpi_aware();
    let mut handles: Vec<HWND> = Vec::new();
    unsafe {
        EnumWindows(
            Some(collect),
            LPARAM(&mut handles as *mut Vec<HWND> as isize),
        )
    }
    .map_err(|e| format!("couldn't list windows: {e}"))?;
    let desktop = unsafe { GetShellWindow() };
    Ok(handles
        .into_iter()
        .filter(|hwnd| *hwnd != desktop && listed(*hwnd))
        .filter_map(describe)
        .collect())
}

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let handles = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
    handles.push(hwnd);
    true.into()
}

/// A window a person sees as an app's: shown, not cloaked (another virtual
/// desktop, a suspended UWP app), not a tool palette.
fn listed(hwnd: HWND) -> bool {
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return false;
    }
    let ex_style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32;
    if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
        return false;
    }
    let mut cloaked = 0u32;
    let read = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut _,
            size_of::<u32>() as u32,
        )
    };
    read.is_err() || cloaked == 0
}

fn describe(hwnd: HWND) -> Option<Window> {
    let title = title(hwnd);
    if title.is_empty() {
        return None;
    }
    let bounds = bounds(hwnd)?;
    let (width, height) = (bounds.right - bounds.left, bounds.bottom - bounds.top);
    if width <= 0 || height <= 0 {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    Some(Window {
        id: hwnd.0 as usize as u64,
        pid,
        app: app_name(pid).unwrap_or_default(),
        title,
        x: bounds.left as f64,
        y: bounds.top as f64,
        width: width as f64,
        height: height as f64,
    })
}

fn title(hwnd: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; len as usize + 1];
    let read = unsafe { GetWindowTextW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..read.max(0) as usize])
}

/// The window's visible frame on screen, without the invisible resize
/// border `GetWindowRect` counts.
fn bounds(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut RECT as *mut _,
            size_of::<RECT>() as u32,
        )
    }
    .or_else(|_| unsafe { GetWindowRect(hwnd, &mut rect) })
    .ok()?;
    Some(rect)
}

/// The process's executable name without `.exe`.
fn app_name(pid: u32) -> Option<String> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let read = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
    };
    let _ = unsafe { CloseHandle(process) };
    read.ok()?;
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    Some(Path::new(&path).file_stem()?.to_string_lossy().into_owned())
}

/// Physical pixels everywhere, whatever each monitor's scale; once per
/// process. Failing (already set, older Windows) leaves the default.
fn dpi_aware() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ =
            unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    });
}

fn hwnd(w: &Window) -> HWND {
    HWND(w.id as usize as *mut _)
}

// ── Pictures ────────────────────────────────────────────────────────────────

/// A PNG of the window as it draws itself, even when covered.
pub fn capture_window(w: &Window) -> Result<Vec<u8>, String> {
    dpi_aware();
    let hwnd = hwnd(w);
    // PrintWindow draws the whole window rect, invisible border included;
    // crop it to the frame `windows` reports.
    let mut outer = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut outer) }.map_err(|e| format!("window is gone: {e}"))?;
    let frame = bounds(hwnd).unwrap_or(outer);
    let picture = grab(
        outer.right - outer.left,
        outer.bottom - outer.top,
        |dc| unsafe { PrintWindow(hwnd, dc, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool() },
    )?;
    let (left, top) = (
        (frame.left - outer.left).max(0),
        (frame.top - outer.top).max(0),
    );
    let width =
        ((frame.right - frame.left) as u32).min(picture.width().saturating_sub(left as u32));
    let height =
        ((frame.bottom - frame.top) as u32).min(picture.height().saturating_sub(top as u32));
    let cropped =
        image::imageops::crop_imm(&picture, left as u32, top as u32, width, height).to_image();
    png(&cropped)
}

/// A PNG of the primary monitor.
pub fn capture_screen() -> Result<Vec<u8>, String> {
    dpi_aware();
    let (width, height) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    let picture = grab(width, height, |dc| unsafe {
        let screen = GetDC(None);
        let copied = BitBlt(
            dc,
            0,
            0,
            width,
            height,
            Some(screen),
            0,
            0,
            SRCCOPY | CAPTUREBLT,
        );
        ReleaseDC(None, screen);
        copied.is_ok()
    })?;
    png(&picture)
}

/// What `draw` paints into a `width`×`height` 32-bit bitmap, as RGBA.
fn grab(width: i32, height: i32, draw: impl FnOnce(HDC) -> bool) -> Result<RgbaImage, String> {
    if width <= 0 || height <= 0 {
        return Err("nothing to capture: zero size".into());
    }
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            // Negative: rows top to bottom.
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    unsafe {
        let dc = CreateCompatibleDC(None);
        let mut bits = std::ptr::null_mut();
        let bitmap = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bitmap) => bitmap,
            Err(e) => {
                let _ = DeleteDC(dc);
                return Err(format!("couldn't make a bitmap: {e}"));
            }
        };
        let previous = SelectObject(dc, bitmap.into());
        let drawn = draw(dc);
        let _ = GdiFlush();
        let len = width as usize * height as usize * 4;
        let mut pixels = std::slice::from_raw_parts(bits as *const u8, len).to_vec();
        SelectObject(dc, previous);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        if !drawn {
            return Err("the window didn't draw itself".into());
        }
        for px in pixels.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
            // GDI leaves alpha undefined.
            px[3] = 255;
        }
        RgbaImage::from_raw(width as u32, height as u32, pixels)
            .ok_or_else(|| "bad bitmap size".into())
    }
}

fn png(picture: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut out = Cursor::new(Vec::new());
    picture
        .write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| format!("couldn't encode PNG: {e}"))?;
    Ok(out.into_inner())
}

// ── Processes ───────────────────────────────────────────────────────────────

/// The process that started `pid`.
pub fn parent(pid: u32) -> Option<u32> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.ok()?;
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut found = None;
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
    while more {
        if entry.th32ProcessID == pid {
            found = Some(entry.th32ParentProcessID);
            break;
        }
        more = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
    }
    let _ = unsafe { CloseHandle(snapshot) };
    found
}

/// Whether `pid` is still running.
pub fn alive(pid: u32) -> bool {
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
    else {
        return false;
    };
    let mut code = 0u32;
    let read = unsafe { GetExitCodeProcess(process, &mut code) };
    let _ = unsafe { CloseHandle(process) };
    read.is_ok() && code == STILL_ACTIVE.0 as u32
}

// ── Input ───────────────────────────────────────────────────────────────────

/// Windows needs no permission to send input.
pub fn input_ready() -> Result<(), String> {
    Ok(())
}

/// Restore the window if minimized and bring it to the foreground; input
/// is only sent once its process owns the foreground.
pub fn focus(w: &Window) -> Result<(), String> {
    dpi_aware();
    let hwnd = hwnd(w);
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        if !SetForegroundWindow(hwnd).as_bool() {
            // Windows only lets the foreground's thread hand it over: borrow
            // that thread's input state for the call.
            let foreground = GetWindowThreadProcessId(GetForegroundWindow(), None);
            let current = GetCurrentThreadId();
            let attached =
                foreground != current && AttachThreadInput(current, foreground, true).as_bool();
            let _ = BringWindowToTop(hwnd);
            let _ = SetForegroundWindow(hwnd);
            if attached {
                let _ = AttachThreadInput(current, foreground, false);
            }
        }
    }
    std::thread::sleep(Duration::from_millis(100));
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid)) };
    if pid != w.pid {
        return Err(format!("couldn't bring {} to the front", w.app));
    }
    Ok(())
}

/// Click `count` times at `x`, `y` in the window.
pub fn click(w: &Window, x: f64, y: f64, button: Button, count: u8) -> Result<(), String> {
    let (down, up) = match button {
        Button::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
        Button::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
        Button::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
    };
    let mut inputs = vec![move_to(w, x, y)];
    for _ in 0..count.max(1) {
        inputs.push(mouse(0, 0, 0, down));
        inputs.push(mouse(0, 0, 0, up));
    }
    send(&inputs)
}

/// Type `text` into the focused control, as Unicode key events.
pub fn type_text(_w: &Window, text: &str) -> Result<(), String> {
    let mut inputs = Vec::new();
    for ch in text.chars() {
        match ch {
            '\r' => {}
            '\n' => press(&mut inputs, VK_RETURN),
            '\t' => press(&mut inputs, VK_TAB),
            _ => {
                let mut units = [0u16; 2];
                for unit in ch.encode_utf16(&mut units) {
                    inputs.push(keyboard(VIRTUAL_KEY(0), *unit, KEYEVENTF_UNICODE));
                    inputs.push(keyboard(
                        VIRTUAL_KEY(0),
                        *unit,
                        KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                    ));
                }
            }
        }
    }
    send(&inputs)
}

/// Press the chord's modifiers, its key, then release in reverse.
pub fn key(_w: &Window, chord: &Chord) -> Result<(), String> {
    let (vk, needs) = virtual_key(chord.key)?;
    let m = chord.modifiers;
    let held: Vec<VIRTUAL_KEY> = [
        (m.cmd, VK_LWIN),
        (m.ctrl || needs.ctrl, VK_CONTROL),
        (m.alt || needs.alt, VK_MENU),
        (m.shift || needs.shift, VK_SHIFT),
    ]
    .into_iter()
    .filter_map(|(on, vk)| on.then_some(vk))
    .collect();
    let mut inputs = Vec::new();
    for vk in &held {
        inputs.push(key_event(*vk, false));
    }
    press(&mut inputs, vk);
    for vk in held.iter().rev() {
        inputs.push(key_event(*vk, true));
    }
    send(&inputs)
}

/// Scroll by `dy` pixels (positive is down) at `x`, `y` in the window.
pub fn scroll(w: &Window, x: f64, y: f64, dy: f64) -> Result<(), String> {
    // One wheel notch is 120 units and scrolls roughly 100 pixels.
    let delta = -(dy / 100. * 120.).round() as i32;
    send(&[
        move_to(w, x, y),
        mouse(0, 0, delta as u32, MOUSEEVENTF_WHEEL),
    ])
}

/// Modifiers a character needs on the current keyboard layout (`?` is
/// shift+/ on US).
#[derive(Default)]
struct Needs {
    shift: bool,
    ctrl: bool,
    alt: bool,
}

fn virtual_key(key: Key) -> Result<(VIRTUAL_KEY, Needs), String> {
    let vk = match key {
        Key::Char(ch) => {
            let mut units = [0u16; 2];
            let [unit] = ch.encode_utf16(&mut units) else {
                return Err(format!("no key types {ch}"));
            };
            let scan = unsafe { VkKeyScanW(*unit) };
            if scan == -1 {
                return Err(format!("no key types {ch} on this keyboard layout"));
            }
            let state = (scan >> 8) as u8;
            let needs = Needs {
                shift: state & 1 != 0,
                ctrl: state & 2 != 0,
                alt: state & 4 != 0,
            };
            return Ok((VIRTUAL_KEY((scan & 0xff) as u16), needs));
        }
        Key::Enter => VK_RETURN,
        Key::Tab => VK_TAB,
        Key::Escape => VK_ESCAPE,
        Key::Backspace => VK_BACK,
        Key::Delete => VK_DELETE,
        Key::Space => VK_SPACE,
        Key::Up => VK_UP,
        Key::Down => VK_DOWN,
        Key::Left => VK_LEFT,
        Key::Right => VK_RIGHT,
        Key::Home => VK_HOME,
        Key::End => VK_END,
        Key::PageUp => VK_PRIOR,
        Key::PageDown => VK_NEXT,
        Key::F(n) if (1..=12).contains(&n) => VIRTUAL_KEY(VK_F1.0 + u16::from(n) - 1),
        Key::F(n) => return Err(format!("no key F{n}")),
    };
    Ok((vk, Needs::default()))
}

/// A key down then up.
fn press(inputs: &mut Vec<INPUT>, vk: VIRTUAL_KEY) {
    inputs.push(key_event(vk, false));
    inputs.push(key_event(vk, true));
}

fn key_event(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    let scan = unsafe { MapVirtualKeyW(u32::from(vk.0), MAPVK_VK_TO_VSC) } as u16;
    let mut flags = if up {
        KEYEVENTF_KEYUP
    } else {
        KEYBD_EVENT_FLAGS(0)
    };
    // Navigation keys sit on the extended keypad; without the flag some
    // apps read them as the numpad's.
    if [
        VK_UP, VK_DOWN, VK_LEFT, VK_RIGHT, VK_HOME, VK_END, VK_PRIOR, VK_NEXT, VK_DELETE, VK_LWIN,
    ]
    .contains(&vk)
    {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    keyboard(vk, scan, flags)
}

fn keyboard(vk: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                ..Default::default()
            },
        },
    }
}

/// A move to `x`, `y` in the window, in the 0–65535 units absolute input
/// uses across the whole virtual desktop.
fn move_to(w: &Window, x: f64, y: f64) -> INPUT {
    let (left, top, width, height) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    let nx = (w.x + x - left as f64) * 65535. / (width - 1).max(1) as f64;
    let ny = (w.y + y - top as f64) * 65535. / (height - 1).max(1) as f64;
    mouse(
        nx.round() as i32,
        ny.round() as i32,
        0,
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
    )
}

fn mouse(dx: i32, dy: i32, data: u32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data,
                dwFlags: flags,
                ..Default::default()
            },
        },
    }
}

fn send(inputs: &[INPUT]) -> Result<(), String> {
    dpi_aware();
    let sent = unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };
    if (sent as usize) < inputs.len() {
        // UIPI drops input to a process running with higher integrity.
        return Err("Windows blocked the input (is the app running as administrator?)".into());
    }
    Ok(())
}
