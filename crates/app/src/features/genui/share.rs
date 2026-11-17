//! What every surface's toolbar offers besides editing: pinning it to the
//! conversation's side panel, and the Export menu (copy to the clipboard,
//! save to a file, ask the agent to add it to the docs, save it as a
//! template). The files themselves come from `gentle_core::genui::export`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gentle_core::genui::export::{self, Format, Palette};
use gentle_core::genui::refs::CodeRef;
use gentle_core::genui::template::Template;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable as _, h_flex};
use gpui_kit::*;
use serde_json::Value;

use super::diagram::DiagramEvent;
use super::store;
use crate::shared::ui;

/// How long a code ref's "is the file there" answer is trusted.
const CHECK_FOR: Duration = Duration::from_secs(5);

/// What a surface knows of where it is shown: pinned to the side panel or
/// not, and the conversation's folder its code refs resolve against.
#[derive(Default)]
pub struct Env {
    pub pinned: bool,
    pub root: Option<PathBuf>,
    /// Whether each resolved ref's file exists, and when that was checked.
    checked: RefCell<HashMap<PathBuf, (Instant, bool)>>,
}

/// Where a code ref leads.
#[derive(Debug, Clone, PartialEq)]
pub enum RefState {
    /// The file, which exists.
    Found(PathBuf),
    /// Inside the folder but not on disk (yet, or anymore).
    Missing(PathBuf),
    /// Outside the conversation's folder.
    Outside,
    /// No folder to resolve against.
    Unknown,
}

impl Env {
    pub fn ref_state(&self, code_ref: &CodeRef) -> RefState {
        let Some(root) = &self.root else {
            return RefState::Unknown;
        };
        let Some(path) = code_ref.resolve(root) else {
            return RefState::Outside;
        };
        let now = Instant::now();
        let mut checked = self.checked.borrow_mut();
        let exists = match checked.get(&path) {
            Some((at, exists)) if now.duration_since(*at) < CHECK_FOR => *exists,
            _ => {
                let exists = path.is_file();
                checked.insert(path.clone(), (now, exists));
                exists
            }
        };
        if exists {
            RefState::Found(path)
        } else {
            RefState::Missing(path)
        }
    }
}

/// What an export starts from: the surface as the user sees it now.
#[derive(Debug, Clone)]
pub struct Source {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub spec: Value,
    /// The conversation's folder: saved files go to its `docs/`.
    pub root: Option<PathBuf>,
}

/// One entry of the Export menu.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Copy(Format),
    Save(Format),
    Docs,
    Template,
}

/// A theme color as CSS hex.
fn hex(color: Hsla) -> String {
    let c = Rgba::from(color);
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(c.r), byte(c.g), byte(c.b))
}

/// Drawn exports in the theme's colors, as the surface looks.
fn palette() -> Palette {
    Palette {
        background: hex(ui::surface()),
        surface: hex(ui::raised()),
        text: hex(ui::fg()),
        muted: hex(ui::muted()),
        border: hex(ui::border_strong()),
        accent: hex(ui::accent()),
        series: [
            ui::info(),
            ui::accent(),
            ui::success(),
            ui::warning(),
            ui::champagne(),
            ui::petal(),
            ui::danger(),
            ui::muted(),
        ]
        .map(hex)
        .to_vec(),
        font: "sans-serif".into(),
    }
}

/// The export's bytes: text, or a PNG drawn from the SVG at 2×.
fn bytes(source: &Source, format: Format) -> Result<Vec<u8>, String> {
    let text = export::render(
        &source.kind,
        &source.title,
        &source.spec,
        format,
        &palette(),
    )?;
    if format != Format::Png {
        return Ok(text.into_bytes());
    }
    let size = svg_size(&text).ok_or("the picture has no size")?;
    crate::shared::backdrop::preview::rasterize_at(
        &text,
        ((size.0 * 2.) as u32, (size.1 * 2.) as u32),
        2.,
    )?
    .encode_png()
    .map_err(|e| e.to_string())
}

/// The `width` and `height` of an SVG's root element.
fn svg_size(svg: &str) -> Option<(f32, f32)> {
    let open = &svg[svg.find("<svg")?..];
    let open = &open[..open.find('>')?];
    let attribute = |name: &str| -> Option<f32> {
        let start = open.find(&format!(" {name}=\""))? + name.len() + 3;
        open[start..].split('"').next()?.parse().ok()
    };
    Some((attribute("width")?, attribute("height")?))
}

/// Where Save starts: the project's `docs/` when there is one.
fn save_dir(root: Option<&Path>) -> PathBuf {
    match root {
        Some(root) if root.join("docs").is_dir() => root.join("docs"),
        Some(root) => root.to_path_buf(),
        None => dirs::home_dir().unwrap_or_default(),
    }
}

/// Run one Export menu entry for the surface `source` describes; what
/// happened comes back as a [`DiagramEvent::Notice`] (a docs request as a
/// [`DiagramEvent::Request`]).
pub fn run<V: EventEmitter<DiagramEvent>>(action: Action, source: Source, cx: &mut Context<V>) {
    let notice = match action {
        Action::Copy(format) => bytes(&source, format).map(|bytes| {
            if format == Format::Png {
                cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(
                    ImageFormat::Png,
                    bytes,
                )));
            } else {
                cx.write_to_clipboard(ClipboardItem::new_string(
                    String::from_utf8_lossy(&bytes).into_owned(),
                ));
            }
            format!("Copied “{}” as {}", source.title, format.name())
        }),
        Action::Save(format) => match bytes(&source, format) {
            Ok(bytes) => {
                save(source, format, bytes, cx);
                return;
            }
            Err(error) => Err(error),
        },
        Action::Docs => {
            match export::docs_command(&source.id, &source.kind, &source.title, &source.spec) {
                Ok(command) => {
                    cx.emit(DiagramEvent::Request(command));
                    return;
                }
                Err(error) => Err(error),
            }
        }
        Action::Template => save_template(&source, cx),
    };
    cx.emit(DiagramEvent::Notice(notice));
}

fn save<V: EventEmitter<DiagramEvent>>(
    source: Source,
    format: Format,
    bytes: Vec<u8>,
    cx: &mut Context<V>,
) {
    let dir = save_dir(source.root.as_deref());
    let name = export::file_name(&source.title, format);
    let picked = cx.prompt_for_new_path(&dir, Some(&name));
    cx.spawn(async move |this, cx| {
        let notice = match picked.await {
            Ok(Ok(Some(path))) => std::fs::write(&path, &bytes)
                .map(|()| format!("Saved {}", path.display()))
                .map_err(|e| format!("Couldn't save {}: {e}", path.display())),
            Ok(Err(error)) => Err(format!("Couldn't ask where to save: {error}")),
            // Cancelled.
            _ => return,
        };
        let _ = this.update(cx, |_, cx| cx.emit(DiagramEvent::Notice(notice)));
    })
    .detach();
}

fn save_template<V>(source: &Source, cx: &mut Context<V>) -> Result<String, String> {
    let dir = store::dir(cx).ok_or("no data folder to keep templates in")?;
    let saved_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let template = Template {
        name: source.title.clone(),
        kind: source.kind.clone(),
        title: source.title.clone(),
        spec: source.spec.clone(),
        saved_at,
    };
    let path = store::save_template(&dir, template).map_err(|e| e.to_string())?;
    let name = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| Template::parse(&text).ok())
        .map_or_else(|| source.title.clone(), |t| t.name);
    Ok(format!(
        "Saved the template “{name}”; insert it with /ui-template"
    ))
}

/// The toolbar's pin button and Export menu for view `V`; `source` reads
/// what the view shows when a menu entry is picked.
pub fn tools<V: EventEmitter<DiagramEvent>>(
    env: &Env,
    kind: &str,
    source: fn(&V) -> Source,
    cx: &mut Context<V>,
) -> impl IntoElement + use<V> {
    let pinned = env.pinned;
    let weak = cx.entity().downgrade();
    let formats = export::formats(kind);
    let has_docs = export::docs_format(kind).is_some();
    let item = move |label: String, icon: Option<IconName>, action: Action| {
        let weak = weak.clone();
        let entry = PopupMenuItem::new(SharedString::from(label));
        match icon {
            Some(icon) => entry.icon(Icon::new(icon)),
            None => entry,
        }
        .on_click(move |_, _, cx| {
            let _ = weak.update(cx, |view, cx| run(action, source(view), cx));
        })
    };
    h_flex()
        .flex_shrink_0()
        .gap_0p5()
        .child(
            Button::new("genui-pin")
                .debug_selector(|| "genui-pin".into())
                .ghost()
                .xsmall()
                .text_color(if pinned { ui::accent() } else { ui::muted() })
                .icon(Icon::new(if pinned {
                    IconName::PinOff
                } else {
                    IconName::Pin
                }))
                .tooltip(if pinned {
                    "Unpin from the side panel"
                } else {
                    "Pin to the side panel"
                })
                .on_click(cx.listener(move |_, _, _, cx| cx.emit(DiagramEvent::Pin(!pinned)))),
        )
        .child(
            Button::new("genui-export")
                .debug_selector(|| "genui-export".into())
                .ghost()
                .xsmall()
                .text_color(ui::muted())
                .icon(Icon::new(IconName::Share2))
                .tooltip("Export")
                .dropdown_menu(move |menu: PopupMenu, window, cx| {
                    let copy_items = item.clone();
                    let save_items = item.clone();
                    let menu = menu
                        .submenu_with_icon(
                            Some(Icon::new(IconName::Copy)),
                            "Copy to clipboard",
                            window,
                            cx,
                            move |mut menu, _, _| {
                                for format in formats {
                                    menu = menu.item(copy_items(
                                        format.name().to_string(),
                                        None,
                                        Action::Copy(*format),
                                    ));
                                }
                                menu
                            },
                        )
                        .submenu_with_icon(
                            Some(Icon::new(IconName::Save)),
                            "Save to file…",
                            window,
                            cx,
                            move |mut menu, _, _| {
                                for format in formats {
                                    menu = menu.item(save_items(
                                        format!("{} (.{})", format.name(), format.extension()),
                                        None,
                                        Action::Save(*format),
                                    ));
                                }
                                menu
                            },
                        );
                    let menu = if has_docs {
                        menu.item(item(
                            "Ask agent to add to docs".into(),
                            Some(IconName::BookOpen),
                            Action::Docs,
                        ))
                    } else {
                        menu
                    };
                    menu.separator().item(item(
                        "Save as template".into(),
                        Some(IconName::LayoutTemplate),
                        Action::Template,
                    ))
                }),
        )
}

#[cfg(test)]
mod tests {
    // Not a glob: GPUI exports a `test` attribute that would shadow `#[test]`.
    use super::{hex, svg_size};
    use gpui_kit::{Hsla, Rgba};

    #[test]
    fn reads_an_svg_size() {
        assert_eq!(
            svg_size(r#"<svg xmlns="x" width="720" height="440" viewBox="0 0 720 440">"#),
            Some((720., 440.))
        );
        assert_eq!(svg_size("<p>"), None);
    }

    #[test]
    fn hex_colors() {
        assert_eq!(
            hex(Hsla::from(Rgba {
                r: 1.,
                g: 0.,
                b: 0.,
                a: 1.
            })),
            "#ff0000"
        );
    }
}
