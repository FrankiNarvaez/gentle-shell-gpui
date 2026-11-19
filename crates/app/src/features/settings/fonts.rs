//! Configuration › Appearance › Fonts: a searchable picker of the installed
//! font families for the interface, the code and the terminal, with
//! ↑/↓/Enter/Esc. Monospace families come first for code and terminal.

use std::collections::HashSet;
use std::rc::Rc;

use gentle_core::fonts;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::*;

use super::SettingsView;
use crate::platform::font_metadata;
use crate::shared::themes::{self, FontSlot};
use crate::shared::ui;

/// `None` is the Default row, shown only with a blank search.
fn picker_entries(
    names: &[String],
    query: &str,
    monospace: Option<&HashSet<String>>,
) -> Vec<Option<String>> {
    let rows = fonts::picker_rows(names, query, monospace.is_some(), |name| {
        monospace.is_some_and(|families| families.contains(&name.to_lowercase()))
    });
    query
        .trim()
        .is_empty()
        .then_some(None)
        .into_iter()
        .chain(rows.into_iter().map(Some))
        .collect()
}

fn entry_cursor(entries: &[Option<String>], selected: Option<&Option<String>>) -> usize {
    entries
        .iter()
        .position(|entry| Some(entry) == selected)
        .unwrap_or(0)
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
    entries: Rc<[Option<String>]>,
    query: String,
    waiting_for_metadata: bool,
    _subscriptions: [Subscription; 2],
}

impl FontsUi {
    pub(super) fn new(window: &mut Window, cx: &mut Context<SettingsView>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search fonts…"));
        let weak = cx.entity().downgrade();
        let subscriptions = [
            cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event
                    && this.fonts.open.is_some()
                {
                    let query = this.fonts.search.read(cx).value().to_string();
                    // Opening the picker also clears the input programmatically.
                    if query != this.fonts.query {
                        this.fonts.query = query;
                        this.fonts.rebuild_entries();
                        this.fonts.select_cursor(0);
                        cx.notify();
                    }
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
            entries: Rc::from([]),
            query: String::new(),
            waiting_for_metadata: false,
            _subscriptions: subscriptions,
        }
    }

    fn rebuild_entries(&mut self) {
        let monospace = match self.open {
            Some(FontSlot::Code | FontSlot::Terminal) => font_metadata::cached_monospace(),
            _ => None,
        };
        self.entries = picker_entries(&self.names, &self.query, monospace).into();
    }

    fn select_cursor(&mut self, cursor: usize) {
        self.cursor = cursor;
        self.scroll.scroll_to_item(cursor);
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
    fn request_font_metadata(&mut self, cx: &mut Context<Self>) {
        if self.fonts.waiting_for_metadata || font_metadata::cached_monospace().is_some() {
            return;
        }
        self.fonts.waiting_for_metadata = true;
        let task = cx.background_executor().spawn(async {
            font_metadata::load_monospace();
        });
        cx.spawn(async move |this, cx| {
            task.await;
            let _ = this.update(cx, |this, cx| {
                let fonts = &mut this.fonts;
                fonts.waiting_for_metadata = false;
                if matches!(fonts.open, Some(FontSlot::Code | FontSlot::Terminal)) {
                    // The user may have searched, navigated, or switched slots
                    // while the metadata was loading. Keep the current row by value.
                    let selected = fonts.entries.get(fonts.cursor).cloned();
                    fonts.rebuild_entries();
                    fonts.select_cursor(entry_cursor(&fonts.entries, selected.as_ref()));
                    cx.notify();
                }
            });
        })
        .detach();
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
            self.fonts.query.clear();
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
            if slot != FontSlot::Interface {
                self.request_font_metadata(cx);
            }
            self.fonts.rebuild_entries();
            self.fonts
                .select_cursor(entry_cursor(&self.fonts.entries, Some(&current)));
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
        let key = keystroke.key.as_str();
        if key == "escape" {
            self.set_fonts_menu(None, window, cx);
            return true;
        }
        if !matches!(key, "up" | "down" | "enter") {
            return false;
        }
        let entries = &self.fonts.entries;
        let n = entries.len();
        let cursor = self.fonts.cursor.min(n.saturating_sub(1));
        match key {
            "up" | "down" if n > 0 => {
                let cursor = if key == "up" {
                    (cursor + n - 1) % n
                } else {
                    (cursor + 1) % n
                };
                self.fonts.select_cursor(cursor);
            }
            "up" | "down" => {}
            "enter" => {
                if let Some(entry) = entries.get(cursor).cloned() {
                    self.choose_font(slot, entry.as_deref(), window, cx);
                }
            }
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
        // Share the cached entries with the popover without copying the list.
        let entries = if open {
            self.fonts.entries.clone()
        } else {
            Rc::from([])
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
                        ui::picker_row(
                            ("font-menu-row", index),
                            index == cursor,
                            chosen,
                            None,
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
                        .on_click(move |_, window, cx| {
                            let family = family.clone();
                            let _ = weak.update(cx, |this, cx| {
                                this.choose_font(slot, family.as_deref(), window, cx)
                            });
                        }),
                    );
                }
                if entries.is_empty() || entries.as_ref() == [None] {
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
                    .child(ui::search_well(&search))
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

#[cfg(test)]
mod tests {
    use super::{HashSet, entry_cursor, picker_entries};

    #[test]
    fn metadata_promotes_monospace_without_moving_selection_to_another_family() {
        let names = ["Menlo".to_string(), "Arial".to_string()];
        let before = picker_entries(&names, "", None);
        assert_eq!(before, [None, Some("Arial".into()), Some("Menlo".into())]);

        let monospace = HashSet::from(["menlo".to_string()]);
        let after = picker_entries(&names, "", Some(&monospace));
        assert_eq!(after, [None, Some("Menlo".into()), Some("Arial".into())]);
        assert_eq!(entry_cursor(&after, before.get(1)), 2);
        assert_eq!(entry_cursor(&after, before.first()), 0);
    }

    #[test]
    fn metadata_respects_the_active_search() {
        let names = ["Arial".to_string(), "Menlo".to_string()];
        let monospace = HashSet::from(["menlo".to_string()]);
        let rows = picker_entries(&names, "ari", Some(&monospace));
        assert_eq!(rows, [Some("Arial".into())]);
        assert!(picker_entries(&names, "zzz", Some(&monospace)).is_empty());
    }
}
