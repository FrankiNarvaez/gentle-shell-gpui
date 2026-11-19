//! Configuration › Appearance › Reading Desk: a collapsible specimen of the
//! font draft. It draws fixed samples with the draft families and sizes only
//! inside this panel, so nothing is applied, saved or re-rendered elsewhere
//! until Apply.

use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, v_flex};
use gpui_kit::*;

use super::SettingsView;
use crate::shared::themes::{FontSlot, Themes};
use crate::shared::ui;

/// Glyph specimens are drawn this much larger than the slot's size.
const MAGNIFY: f32 = 1.65;

impl SettingsView {
    pub(super) fn reading_desk(&self, cx: &mut Context<Self>) -> Div {
        let open = self.fonts.desk_open;
        let toggle = div()
            .id("reading-desk-toggle")
            .flex()
            .items_center()
            .gap_2()
            .py_1()
            .cursor_pointer()
            .text_sm()
            .text_color(ui::champagne())
            .child(Icon::new(if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            }))
            .child("Reading Desk")
            .child(
                div()
                    .ml_auto()
                    .text_xs()
                    .text_color(ui::faint())
                    .child("Draft preview"),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.fonts.desk_open = !this.fonts.desk_open;
                cx.notify();
            }));
        let mut desk = v_flex()
            .mb_2()
            .px_3()
            .py_2()
            .rounded_lg()
            .bg(ui::well())
            .child(toggle);
        if open {
            let interface = ui::default_interface(cx);
            let zoom = f32::from(cx.global::<Themes>().interface_size) / 100.;
            desk = desk.child(self.desk_body(&interface, zoom));
        }
        desk
    }

    fn desk_body(&self, default_interface: &SharedString, zoom: f32) -> Div {
        let draft = &self.fonts.draft;
        let family = |slot| draft.effective_family(slot, default_interface);
        let size = |slot| px(draft.size(slot) * zoom);
        let (ui_family, code_family, term_family) = (
            family(FontSlot::Interface),
            family(FontSlot::Code),
            family(FontSlot::Terminal),
        );
        let (ui_size, code_size, term_size) = (
            size(FontSlot::Interface),
            size(FontSlot::Code),
            size(FontSlot::Terminal),
        );
        let glyph =
            |label: &'static str, text: &'static str, family: SharedString, size: Pixels| {
                v_flex().gap_1().child(ui::section_label(label)).child(
                    div()
                        .font_family(family)
                        .text_size(size * MAGNIFY)
                        .text_color(ui::champagne())
                        .child(text),
                )
            };
        let sample = |label: &'static str, family: SharedString, size: Pixels, body: Div| {
            v_flex().gap_1().child(ui::section_label(label)).child(
                body.font_family(family)
                    .text_size(size)
                    .line_height(size * 1.45)
                    .text_color(ui::fg()),
            )
        };
        let specimens = v_flex()
            .w(rems(150. / 16.))
            .flex_shrink_0()
            .gap_3()
            .p_3()
            .rounded_md()
            .bg(ui::raised())
            .child(glyph("Interface", "Aa Éé", ui_family.clone(), ui_size))
            .child(glyph("Code", "O0 Il1", code_family.clone(), code_size))
            .child(glyph("Terminal", "❯ ~/", term_family.clone(), term_size));
        let samples = v_flex()
            .flex_1()
            .min_w_0()
            .gap_3()
            .child(sample(
                "Conversation",
                ui_family,
                ui_size,
                div().child("Résumé ready. Review the latest changes. Café · acción · 0123456789"),
            ))
            .child(sample(
                "Editor · preview.rs",
                code_family,
                code_size,
                div().child("let id = \"O0Il1\"; // 1 l I | 0 O"),
            ))
            .child(sample(
                "Terminal · zsh",
                term_family,
                term_size,
                v_flex()
                    .child("~/studio ❯ git diff --stat")
                    .child(" fonts.rs | 12 +++++"),
            ));
        div().flex().gap_4().pt_2().child(specimens).child(samples)
    }
}
