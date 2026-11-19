//! Configuration › Appearance › Fonts: a searchable picker of the installed
//! font families for the interface, the code and the terminal, with
//! ↑/↓/Enter/Esc. Monospace families come first for code and terminal.

use std::collections::HashSet;
use std::rc::Rc;

use gentle_core::fonts::{self, MAX_SIZE, MIN_SIZE};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
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

/// What a size field's text means.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum SizeInput {
    Valid(f32),
    /// Still being typed: empty, `12.`, or a prefix of a valid size (`1`–`3`).
    Incomplete,
    /// Not a number, or out of range.
    Invalid,
}

pub(super) fn parse_size(text: &str) -> SizeInput {
    let text = text.trim();
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    if text.is_empty() {
        return SizeInput::Incomplete;
    }
    if let Some(whole) = text.strip_suffix('.') {
        return if !whole.is_empty() && digits(whole) {
            SizeInput::Incomplete
        } else {
            SizeInput::Invalid
        };
    }
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty() || !digits(whole) || !digits(fraction) {
        return SizeInput::Invalid;
    }
    if fraction.is_empty() && matches!(whole, "1" | "2" | "3") {
        return SizeInput::Incomplete;
    }
    match text.parse::<f32>() {
        Ok(size) if (MIN_SIZE..=MAX_SIZE).contains(&size) => SizeInput::Valid(size),
        _ => SizeInput::Invalid,
    }
}

fn format_size(size: f32) -> String {
    if size.fract() == 0. {
        format!("{size:.0}")
    } else {
        format!("{size}")
    }
}

fn slot_index(slot: FontSlot) -> usize {
    match slot {
        FontSlot::Interface => 0,
        FontSlot::Code => 1,
        FontSlot::Terminal => 2,
    }
}

/// The families and sizes being edited in the settings screen. Nothing here
/// reaches the app until the parent applies `to_apply`; dropping the screen
/// drops the draft.
pub(super) struct Draft {
    baseline: themes::Fonts,
    fonts: themes::Fonts,
    texts: [String; 3],
}

impl Draft {
    pub(super) fn new(baseline: themes::Fonts) -> Self {
        let texts = FontSlot::ALL.map(|slot| format_size(baseline.size(slot)));
        Self {
            fonts: baseline.clone(),
            baseline,
            texts,
        }
    }

    pub(super) fn family(&self, slot: FontSlot) -> Option<&SharedString> {
        self.fonts.get(slot)
    }

    /// The last valid size of `slot`: what the preview shows while the text
    /// is being edited.
    pub(super) fn size(&self, slot: FontSlot) -> f32 {
        self.fonts.size(slot)
    }

    pub(super) fn text(&self, slot: FontSlot) -> &str {
        &self.texts[slot_index(slot)]
    }

    pub(super) fn input(&self, slot: FontSlot) -> SizeInput {
        parse_size(self.text(slot))
    }

    pub(super) fn set_family(&mut self, slot: FontSlot, family: Option<&str>) {
        self.fonts.set(
            slot,
            family.map(|name| SharedString::from(name.to_string())),
        );
    }

    pub(super) fn set_text(&mut self, slot: FontSlot, text: &str) {
        self.texts[slot_index(slot)] = text.to_string();
        if let SizeInput::Valid(size) = parse_size(text) {
            self.fonts.set_size(slot, size);
        }
    }

    /// The family the preview draws `slot` with: its draft family, else its
    /// default; the terminal's default is the draft code font.
    pub(super) fn effective_family(
        &self,
        slot: FontSlot,
        default_interface: &SharedString,
    ) -> SharedString {
        match (self.family(slot), slot) {
            (Some(family), _) => family.clone(),
            (None, FontSlot::Interface) => default_interface.clone(),
            (None, FontSlot::Code) => ui::default_mono(),
            (None, FontSlot::Terminal) => self
                .fonts
                .terminal_family()
                .cloned()
                .unwrap_or_else(ui::default_mono),
        }
    }

    pub(super) fn dirty(&self) -> bool {
        self.fonts != self.baseline
            || FontSlot::ALL
                .iter()
                .any(|&slot| self.input(slot) != SizeInput::Valid(self.baseline.size(slot)))
    }

    /// The first size problem, `Invalid` before `Incomplete`.
    pub(super) fn problem(&self) -> Option<SizeInput> {
        let inputs = FontSlot::ALL.map(|slot| self.input(slot));
        inputs
            .iter()
            .find(|input| **input == SizeInput::Invalid)
            .or_else(|| inputs.iter().find(|input| **input == SizeInput::Incomplete))
            .copied()
    }

    pub(super) fn can_apply(&self) -> bool {
        self.dirty() && self.problem().is_none()
    }

    /// The fonts to apply, or `None` when clean or a size is not valid.
    pub(super) fn to_apply(&self) -> Option<themes::Fonts> {
        self.can_apply().then(|| self.fonts.clone())
    }

    /// Make `fonts` (what the app uses now) the clean state.
    pub(super) fn rebase(&mut self, fonts: themes::Fonts) {
        *self = Self::new(fonts);
    }

    pub(super) fn discard(&mut self) {
        let baseline = self.baseline.clone();
        self.rebase(baseline);
    }
}

/// The picker's state: which slot's list is open, the search, the cursor.
pub(super) struct FontsUi {
    /// Edits not yet applied; recreated with the settings screen.
    pub(super) draft: Draft,
    /// One number field per slot, in `FontSlot::ALL` order.
    sizes: [Entity<InputState>; 3],
    /// Whether the Reading Desk is expanded.
    pub(super) desk_open: bool,
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
    _subscriptions: Vec<Subscription>,
}

impl FontsUi {
    pub(super) fn new(window: &mut Window, cx: &mut Context<SettingsView>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search fonts…"));
        let weak = cx.entity().downgrade();
        let draft = Draft::new(cx.global::<themes::Themes>().fonts.clone());
        let sizes = FontSlot::ALL.map(|slot| {
            let text = draft.text(slot).to_string();
            cx.new(|cx| InputState::new(window, cx).default_value(text))
        });
        let mut subscriptions = vec![
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
        for (slot, input) in FontSlot::ALL.into_iter().zip(&sizes) {
            subscriptions.push(
                cx.subscribe(input, move |this, input, event: &InputEvent, cx| {
                    if let InputEvent::Change = event {
                        let text = input.read(cx).value().to_string();
                        this.fonts.draft.set_text(slot, &text);
                        cx.notify();
                    }
                }),
            );
        }
        Self {
            draft,
            sizes,
            desk_open: true,
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
            let current = self.fonts.draft.family(slot).map(|f| f.to_string());
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
        self.fonts.draft.set_family(slot, family);
        cx.notify();
    }

    fn sync_size_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (slot, input) in FontSlot::ALL.into_iter().zip(&self.fonts.sizes) {
            let text = self.fonts.draft.text(slot).to_string();
            input.update(cx, |state, cx| {
                if state.value().as_ref() != text {
                    state.set_value(text, window, cx);
                }
            });
        }
    }

    /// Commit the draft once through the backend; no-op when clean or invalid.
    fn apply_font_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(fonts) = self.fonts.draft.to_apply() else {
            return;
        };
        themes::apply_fonts(fonts, cx);
        // Rebase on what the app really uses: an uninstalled family is dropped.
        let applied = cx.global::<themes::Themes>().fonts.clone();
        self.fonts.draft.rebase(applied);
        self.sync_size_inputs(window, cx);
        cx.notify();
    }

    fn discard_font_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.fonts.draft.discard();
        self.sync_size_inputs(window, cx);
        cx.notify();
    }

    /// Apply and Discard for the draft, with what blocks applying.
    pub(super) fn font_actions(&self, cx: &mut Context<Self>) -> Div {
        let draft = &self.fonts.draft;
        let dirty = draft.dirty();
        let (note, color) = match draft.problem() {
            Some(SizeInput::Invalid) => (
                format!(
                    "Sizes are numbers from {} to {} px.",
                    MIN_SIZE as u32, MAX_SIZE as u32
                ),
                ui::danger(),
            ),
            Some(_) => ("Finish the size to apply.".to_string(), ui::muted()),
            None if dirty => ("Not applied yet.".to_string(), ui::muted()),
            None => (String::new(), ui::muted()),
        };
        h_flex()
            .pb_2()
            .gap_2()
            .items_center()
            .child(
                Button::new("fonts-apply")
                    .small()
                    .primary()
                    .label("Apply")
                    .disabled(!draft.can_apply())
                    .on_click(cx.listener(|this, _, window, cx| this.apply_font_draft(window, cx))),
            )
            .child(
                Button::new("fonts-discard")
                    .small()
                    .label("Discard")
                    .disabled(!dirty)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.discard_font_draft(window, cx)),
                    ),
            )
            .child(div().text_xs().text_color(color).child(note))
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
                    .child(self.size_field(slot))
            })
            .collect()
    }

    fn size_field(&self, slot: FontSlot) -> impl IntoElement {
        let input = &self.fonts.sizes[slot_index(slot)];
        let invalid = self.fonts.draft.input(slot) == SizeInput::Invalid;
        h_flex()
            .flex_shrink_0()
            .gap_1()
            .items_center()
            .child(
                div()
                    .w(rems(64. / 16.))
                    .rounded_md()
                    .when(invalid, |d| d.border_1().border_color(ui::danger()))
                    .child(Input::new(input).small()),
            )
            .child(div().text_xs().text_color(ui::muted()).child("px"))
    }

    fn font_picker(&self, slot: FontSlot, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        let open = self.fonts.open == Some(slot);
        let current = self.fonts.draft.family(slot).map(|f| f.to_string());
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
    use super::{Draft, FontSlot, HashSet, SizeInput, entry_cursor, parse_size, picker_entries};
    use crate::shared::themes::Fonts;
    use crate::shared::ui;

    fn draft() -> Draft {
        Draft::new(Fonts::default())
    }

    #[test]
    fn size_text_distinguishes_valid_intermediate_and_invalid() {
        assert_eq!(parse_size("14"), SizeInput::Valid(14.));
        assert_eq!(parse_size(" 8.5 "), SizeInput::Valid(8.5));
        assert_eq!(parse_size("8"), SizeInput::Valid(8.));
        assert_eq!(parse_size("32"), SizeInput::Valid(32.));
        for typing in ["", "  ", "1", "2", "3", "12."] {
            assert_eq!(parse_size(typing), SizeInput::Incomplete, "{typing:?}");
        }
        for bad in [
            "7", "33", "0", "99", "abc", "-14", "+14", "1e1", ".5", "1.2.3", "NaN", "inf", "14px",
        ] {
            assert_eq!(parse_size(bad), SizeInput::Invalid, "{bad:?}");
        }
    }

    #[test]
    fn clean_draft_has_nothing_to_apply_or_discard() {
        let draft = draft();
        assert!(!draft.dirty());
        assert!(!draft.can_apply());
        assert_eq!(draft.to_apply(), None);
        assert_eq!(draft.problem(), None);
    }

    #[test]
    fn editing_the_draft_never_touches_the_baseline_until_rebased() {
        let mut draft = draft();
        draft.set_family(FontSlot::Code, Some("Candidate"));
        draft.set_text(FontSlot::Terminal, "18");
        assert!(draft.dirty());
        let pending = draft.to_apply().expect("valid edits apply");
        assert_eq!(
            pending.get(FontSlot::Code).map(|f| f.as_ref()),
            Some("Candidate")
        );
        assert_eq!(pending.size(FontSlot::Terminal), 18.);
        assert_eq!(
            pending.size(FontSlot::Code),
            Fonts::default().size(FontSlot::Code)
        );
        draft.rebase(pending);
        assert!(!draft.dirty());
        assert_eq!(draft.text(FontSlot::Terminal), "18");
    }

    #[test]
    fn discard_restores_families_sizes_and_text() {
        let mut draft = draft();
        let text = draft.text(FontSlot::Interface).to_string();
        draft.set_family(FontSlot::Interface, Some("Candidate"));
        draft.set_text(FontSlot::Interface, "abc");
        assert!(draft.dirty());
        draft.discard();
        assert!(!draft.dirty());
        assert_eq!(draft.family(FontSlot::Interface), None);
        assert_eq!(draft.text(FontSlot::Interface), text);
        assert_eq!(draft.to_apply(), None);
    }

    #[test]
    fn invalid_or_unfinished_sizes_are_dirty_but_not_applicable() {
        let mut draft = draft();
        draft.set_family(FontSlot::Code, Some("Candidate"));
        let size = draft.size(FontSlot::Interface);
        draft.set_text(FontSlot::Interface, "99");
        assert!(draft.dirty());
        assert_eq!(draft.problem(), Some(SizeInput::Invalid));
        assert!(!draft.can_apply());
        assert_eq!(draft.to_apply(), None);
        // The preview keeps the last valid size meanwhile.
        assert_eq!(draft.size(FontSlot::Interface), size);
        draft.set_text(FontSlot::Interface, "");
        assert_eq!(draft.problem(), Some(SizeInput::Incomplete));
        assert!(!draft.can_apply());
        // Invalid text alone, with families unchanged, still offers Discard.
        let mut only_text = self::draft();
        only_text.set_text(FontSlot::Code, "x");
        assert!(only_text.dirty() && !only_text.can_apply());
        // Returning to the original value is clean again.
        let mut back = self::draft();
        let original = back.text(FontSlot::Code).to_string();
        back.set_text(FontSlot::Code, "x");
        back.set_text(FontSlot::Code, &original);
        assert!(!back.dirty());
    }

    #[test]
    fn terminal_preview_inherits_only_the_draft_code_family() {
        let interface = "Kit Interface".into();
        let mut draft = draft();
        assert_eq!(
            draft.effective_family(FontSlot::Interface, &interface),
            interface
        );
        assert_eq!(
            draft.effective_family(FontSlot::Code, &interface),
            ui::default_mono()
        );
        assert_eq!(
            draft.effective_family(FontSlot::Terminal, &interface),
            ui::default_mono()
        );
        draft.set_family(FontSlot::Code, Some("Candidate"));
        draft.set_text(FontSlot::Code, "20");
        assert_eq!(
            draft.effective_family(FontSlot::Terminal, &interface),
            "Candidate"
        );
        assert_ne!(draft.size(FontSlot::Terminal), 20.);
        draft.set_family(FontSlot::Terminal, Some("Own"));
        assert_eq!(
            draft.effective_family(FontSlot::Terminal, &interface),
            "Own"
        );
    }

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
