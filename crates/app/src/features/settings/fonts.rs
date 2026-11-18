//! Configuration › Appearance › Fonts: a searchable picker of the installed
//! font families for the interface, the code and the terminal, with
//! ↑/↓/Enter/Esc. Monospace families come first for code and terminal.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use gentle_core::fonts;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::SettingsView;
use crate::shared::themes::{self, FontSlot};
use crate::shared::ui;

/// Whether each family is monospace, asked of the text system once per
/// session: the picker never measures a family twice.
static MONOSPACE_CACHE: LazyLock<Mutex<HashMap<String, bool>>> = LazyLock::new(Mutex::default);

/// A family is monospace when 'i' and 'W' advance the same.
fn is_monospace(family: &str, cx: &App) -> bool {
    if let Some(known) = MONOSPACE_CACHE
        .lock()
        .ok()
        .and_then(|m| m.get(family).copied())
    {
        return known;
    }
    let text_system = cx.text_system();
    let id = text_system.resolve_font(&font(SharedString::from(family.to_string())));
    let size = px(16.);
    let width = |c| text_system.advance(id, size, c).map(|s| s.width);
    let mono = match (width('i'), width('W')) {
        (Ok(i), Ok(w)) => (f32::from(i) - f32::from(w)).abs() < 0.01,
        _ => false,
    };
    if let Ok(mut cache) = MONOSPACE_CACHE.lock() {
        cache.insert(family.to_string(), mono);
    }
    mono
}

/// The picker's state: which slot's list is open, the search, the cursor.
pub(super) struct FontsUi {
    open: Option<FontSlot>,
    search: Entity<InputState>,
    /// Index into the entries: the Default entry (when shown), then families.
    cursor: usize,
    scroll: ScrollHandle,
    /// The installed families, read when the picker opens.
    names: Vec<String>,
    _subscriptions: [Subscription; 2],
}

impl FontsUi {
    pub(super) fn new(window: &mut Window, cx: &mut Context<SettingsView>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search fonts…"));
        let weak = cx.entity().downgrade();
        let subscriptions = [
            cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    this.fonts.cursor = 0;
                    this.fonts.scroll.scroll_to_item(0);
                    cx.notify();
                }
            }),
            // ↑/↓/Enter/Esc in the open picker beat its focused search field
            // and the workspace's Esc, which would close the whole screen.
            cx.intercept_keystrokes(move |event, window, cx| {
                let consumed = weak
                    .update(cx, |this, cx| this.fonts_key(&event.keystroke, window, cx))
                    .unwrap_or(false);
                if consumed {
                    cx.stop_propagation();
                }
            }),
        ];
        Self {
            open: None,
            search,
            cursor: 0,
            scroll: ScrollHandle::new(),
            names: Vec::new(),
            _subscriptions: subscriptions,
        }
    }
}

fn slot_label(slot: FontSlot) -> &'static str {
    match slot {
        FontSlot::Interface => "Interface font",
        FontSlot::Code => "Code font",
        FontSlot::Terminal => "Terminal font",
    }
}

/// What the Default entry says for `slot`.
fn default_label(slot: FontSlot) -> &'static str {
    match slot {
        FontSlot::Terminal => "Same as code font",
        _ => "Default",
    }
}

impl SettingsView {
    /// The picker's entries for the current search: `None` is the Default
    /// entry (only while not searching), then the matching families.
    fn font_entries(&self, slot: FontSlot, cx: &App) -> Vec<Option<String>> {
        let query = self.fonts.search.read(cx).value().to_string();
        let mut entries = Vec::new();
        if query.trim().is_empty() {
            entries.push(None);
        }
        let mono_first = slot != FontSlot::Interface;
        entries.extend(
            fonts::picker_rows(&self.fonts.names, &query, mono_first, |name| {
                is_monospace(name, cx)
            })
            .into_iter()
            .map(Some),
        );
        entries
    }

    /// Open the picker for `slot` (closing any other), or close it.
    fn set_fonts_menu(
        &mut self,
        slot: Option<FontSlot>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.fonts.open == slot {
            return;
        }
        self.fonts.open = slot;
        if let Some(slot) = slot {
            self.fonts.names = cx.text_system().all_font_names();
            self.fonts.search.update(cx, |state, cx| {
                state.set_value("", window, cx);
                state.focus(window, cx);
            });
            // Start on the slot's own family, so Enter keeps it.
            let current = cx
                .global::<themes::Themes>()
                .fonts
                .get(slot)
                .map(|f| f.to_string());
            let entries = self.font_entries(slot, cx);
            self.fonts.cursor = entries.iter().position(|e| *e == current).unwrap_or(0);
            self.fonts.scroll.scroll_to_item(self.fonts.cursor);
        }
        cx.notify();
    }

    /// ↑/↓/Enter/Esc while a fonts picker is open.
    fn fonts_key(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(slot) = self.fonts.open else {
            return false;
        };
        let m = &keystroke.modifiers;
        if m.platform || m.control || m.alt {
            return false;
        }
        let entries = self.font_entries(slot, cx);
        let n = entries.len();
        let cursor = self.fonts.cursor.min(n.saturating_sub(1));
        match keystroke.key.as_str() {
            "up" | "down" if n > 0 => {
                self.fonts.cursor = if keystroke.key == "up" {
                    (cursor + n - 1) % n
                } else {
                    (cursor + 1) % n
                };
                self.fonts.scroll.scroll_to_item(self.fonts.cursor);
            }
            "up" | "down" => {}
            "enter" => {
                if let Some(entry) = entries.get(cursor) {
                    self.choose_font(slot, entry.as_deref(), window, cx);
                }
            }
            "escape" => self.set_fonts_menu(None, window, cx),
            _ => return false,
        }
        cx.notify();
        true
    }

    fn choose_font(
        &mut self,
        slot: FontSlot,
        family: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_fonts_menu(None, window, cx);
        themes::set_font(slot, family, cx);
    }

    /// The Fonts rows: a button per slot showing its family, opening its picker.
    pub(super) fn fonts_rows(&self, cx: &mut Context<Self>) -> Vec<Div> {
        FontSlot::ALL
            .into_iter()
            .map(|slot| {
                h_flex()
                    .pb_2()
                    .gap_3()
                    .child(
                        div()
                            .w(rems(120. / 16.))
                            .text_sm()
                            .text_color(ui::muted())
                            .child(slot_label(slot)),
                    )
                    .child(div().flex_1().min_w_0().child(self.font_picker(slot, cx)))
            })
            .collect()
    }

    fn font_picker(&self, slot: FontSlot, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        let open = self.fonts.open == Some(slot);
        let current = cx
            .global::<themes::Themes>()
            .fonts
            .get(slot)
            .map(|f| f.to_string());
        let label = current
            .clone()
            .unwrap_or_else(|| default_label(slot).to_string());
        let trigger = Button::new(SharedString::from(format!("font-{}", slot.key())))
            .small()
            .ghost()
            .label(label)
            .text_color(if current.is_some() {
                ui::fg()
            } else {
                ui::muted()
            })
            .dropdown_caret(true);
        // Only the open picker builds its rows.
        let entries = if open {
            self.font_entries(slot, cx)
        } else {
            Vec::new()
        };
        let cursor = self.fonts.cursor;
        let search = self.fonts.search.clone();
        let scroll = self.fonts.scroll.clone();
        let toggle = weak.clone();

        Popover::new(SharedString::from(format!("font-menu-{}", slot.key())))
            .anchor(Anchor::TopLeft)
            .trigger(trigger)
            .open(open)
            .track_focus(&self.fonts.search.focus_handle(cx))
            .on_open_change(move |open, window, cx| {
                let slot = open.then_some(slot);
                let _ = toggle.update(cx, |this, cx| this.set_fonts_menu(slot, window, cx));
            })
            .w(rems(360. / 16.))
            .appearance(false)
            .content(move |_, _, cx| {
                let mut list = v_flex()
                    .id("font-menu-list")
                    .max_h(rems(320. / 16.))
                    .overflow_y_scroll()
                    .track_scroll(&scroll)
                    .px(rems(6. / 16.))
                    .pb(rems(6. / 16.));
                for (index, entry) in entries.iter().enumerate() {
                    let chosen = *entry == current;
                    let title: SharedString = match entry {
                        Some(name) => name.clone().into(),
                        None => default_label(slot).into(),
                    };
                    let family = entry.clone();
                    let weak = weak.clone();
                    list = list.child(
                        h_flex()
                            .id(("font-menu-row", index))
                            .w_full()
                            .gap_2()
                            .px_3()
                            .py_1p5()
                            .rounded(rems(ui::ISLAND_ROW / 16.))
                            .cursor_pointer()
                            .hover(|s| s.bg(ui::surface_hover()))
                            .when(index == cursor, |d| d.bg(ui::surface_hover()))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(rems(13. / 16.))
                                    .text_color(match (chosen, entry) {
                                        (true, _) => ui::accent(),
                                        (_, Some(_)) => ui::fg(),
                                        _ => ui::muted(),
                                    })
                                    .child(title),
                            )
                            .when(chosen, |d| {
                                d.child(
                                    Icon::new(IconName::Check).xsmall().text_color(ui::accent()),
                                )
                            })
                            .on_click(move |_, window, cx| {
                                let family = family.clone();
                                let _ = weak.update(cx, |this, cx| {
                                    this.choose_font(slot, family.as_deref(), window, cx)
                                });
                            }),
                    );
                }
                if entries.iter().all(Option::is_none) {
                    list = list.child(
                        div()
                            .px_3()
                            .py_1p5()
                            .text_xs()
                            .text_color(ui::faint())
                            .child("No installed font matches."),
                    );
                }
                let content = v_flex()
                    .w_full()
                    .font_family(cx.theme().font_family.clone())
                    .text_color(ui::fg())
                    .child(
                        div().p(rems(6. / 16.)).child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .h(rems(36. / 16.))
                                .px_3()
                                .rounded(rems(ui::ISLAND_ROW / 16.))
                                .bg(ui::well())
                                .child(Icon::new(IconName::Search).xsmall().text_color(ui::faint()))
                                .child(div().flex_1().child(Input::new(&search).appearance(false))),
                        ),
                    )
                    .child(list);
                ui::island(cx)
                    .w_full()
                    .child(ui::popover_in(
                        ElementId::Name("font-menu-in".into()),
                        content,
                    ))
                    .into_any_element()
            })
    }
}
