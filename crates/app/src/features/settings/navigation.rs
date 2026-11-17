//! Category navigation for Configuration. Existing settings retain their own
//! state and persistence; this module owns only navigation and search metadata.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::SettingsView;
use crate::shared::ui;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum SettingsPage {
    #[default]
    General,
    Appearance,
    Agents,
    Models,
    Devices,
    Integrations,
    Notifications,
    Updates,
    Advanced,
}

impl SettingsPage {
    pub const ALL: [Self; 9] = [
        Self::General,
        Self::Appearance,
        Self::Agents,
        Self::Models,
        Self::Devices,
        Self::Integrations,
        Self::Notifications,
        Self::Updates,
        Self::Advanced,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::General => "settings-nav-general",
            Self::Appearance => "settings-nav-appearance",
            Self::Agents => "settings-nav-agents",
            Self::Models => "settings-nav-models",
            Self::Devices => "settings-nav-devices",
            Self::Integrations => "settings-nav-integrations",
            Self::Notifications => "settings-nav-notifications",
            Self::Updates => "settings-nav-updates",
            Self::Advanced => "settings-nav-advanced",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Agents => "Agents",
            Self::Models => "Models",
            Self::Devices => "Devices",
            Self::Integrations => "Integrations",
            Self::Notifications => "Notifications",
            Self::Updates => "Updates",
            Self::Advanced => "Advanced",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::General => "Environment and setup",
            Self::Appearance => "Theme and window appearance",
            Self::Agents => "Agent behavior and background work",
            Self::Models => "Model and profile defaults",
            Self::Devices => "Connected development devices",
            Self::Integrations => "Companions and external tools",
            Self::Notifications => "Alerts and notification preferences",
            Self::Updates => "Application and tool updates",
            Self::Advanced => "Installed packages and extensions",
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            Self::General => IconName::Settings,
            Self::Appearance => IconName::Sparkles,
            Self::Agents => IconName::ShieldCheck,
            Self::Models => IconName::Cpu,
            Self::Devices => IconName::Smartphone,
            Self::Integrations => IconName::Bot,
            Self::Notifications => IconName::Bell,
            Self::Updates => IconName::RefreshCw,
            Self::Advanced => IconName::Layers,
        }
    }

    /// Search covers the visible category and commonly used setting names.
    /// It only locates a page; it never changes a setting or its storage.
    fn keywords(self) -> &'static str {
        match self {
            Self::General => "home setup requirements agent directory configuration launcher path",
            Self::Appearance => "theme color dark light import share transparency window opacity interface zoom size fonts",
            Self::Agents => "background subagents gentle ai hydration water reminders permissions",
            Self::Models => "model provider thinking effort profile default scoped",
            Self::Devices => "devices device discovery phone android emulator switches",
            Self::Integrations => "claude bridge maestro music gentle stream external tools integrations",
            Self::Notifications => "notification notifications sounds alert toggle status",
            Self::Updates => "update upgrade versions app gentle shell engram release install",
            Self::Advanced => "packages extensions install remove npm git package management",
        }
    }

    pub fn matches_query(self, query: &str) -> bool {
        let searchable = format!(
            "{} {} {}",
            self.label(),
            self.description(),
            self.keywords()
        )
        .to_lowercase();

        query
            .split_whitespace()
            .all(|word| searchable.contains(&word.to_lowercase()))
    }
}

fn search_field(
    filter: &Entity<InputState>,
    query: &str,
    cx: &mut Context<SettingsView>,
) -> Div {
    ui::field()
        .w_full()
        .child(Icon::new(IconName::Search).small().text_color(ui::faint()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(Input::new(filter).appearance(false).small()),
        )
        .when(!query.is_empty(), |field| {
            field.child(
                Button::new("settings-search-clear")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::X))
                    .tooltip("Clear settings search")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.settings_search.update(cx, |state, cx| {
                            state.set_value("", window, cx);
                        });
                        cx.notify();
                    })),
            )
        })
}

fn navigation_items(
    selected: SettingsPage,
    query: &str,
    cx: &mut Context<SettingsView>,
) -> Div {
    let matches: Vec<_> = SettingsPage::ALL
        .into_iter()
        .filter(|page| page.matches_query(query))
        .collect();

    v_flex()
        .gap_1()
        .when(matches.is_empty(), |list| {
            list.child(
                div()
                    .p_3()
                    .text_xs()
                    .text_color(ui::muted())
                    .child("No matching settings. Try another term."),
            )
        })
        .children(matches.into_iter().map(|page| {
            let active = page == selected;
            div()
                .id(page.id())
                .w_full()
                .rounded_md()
                .cursor_pointer()
                .hover(|row| row.bg(ui::surface_hover()))
                .when(active, |row| row.bg(ui::accent().opacity(0.12)))
                .on_click(cx.listener(move |this, _, _, cx| this.navigate_to(page, cx)))
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_2()
                        .child(
                            div()
                                .w(rems(3. / 16.))
                                .h(rems(20. / 16.))
                                .rounded_full()
                                .bg(if active {
                                    ui::accent()
                                } else {
                                    gpui_kit::transparent_black()
                                }),
                        )
                        .child(
                            Icon::new(page.icon())
                                .small()
                                .text_color(if active { ui::accent() } else { ui::muted() }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_sm()
                                .text_color(if active { ui::fg() } else { ui::muted() })
                                .when(active, |title| title.font_weight(FontWeight::SEMIBOLD))
                                .child(page.label()),
                        ),
                )
        }))
}

/// Persistent desktop navigation with inline search.
pub(super) fn sidebar(
    selected: SettingsPage,
    filter: &Entity<InputState>,
    cx: &mut Context<SettingsView>,
) -> impl IntoElement {
    let query = filter.read(cx).value().to_string();

    v_flex()
        .id("settings-navigation")
        .w(rems(220. / 16.))
        .h_full()
        .flex_shrink_0()
        .min_h_0()
        .border_r_1()
        .border_color(ui::border())
        .child(
            v_flex()
                .px_3()
                .pt_4()
                .pb_3()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(ui::muted())
                        .child("CATEGORIES"),
                )
                .child(search_field(filter, &query, cx)),
        )
        .child(
            v_flex()
                .id("settings-navigation-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px_2()
                .pb_2()
                .child(navigation_items(selected, &query, cx)),
        )
}

/// When the window is narrow, replace the left rail with an inline selector.
/// The same search field and navigation actions are reused in both layouts.
pub(super) fn compact(
    selected: SettingsPage,
    filter: &Entity<InputState>,
    expanded: bool,
    cx: &mut Context<SettingsView>,
) -> Div {
    let query = filter.read(cx).value().to_string();

    v_flex()
        .gap_2()
        .px_3()
        .pt_3()
        .pb_2()
        .border_b_1()
        .border_color(ui::border())
        .child(
            Button::new("settings-compact-picker")
                .ghost()
                .small()
                .icon(Icon::new(IconName::ChevronDown))
                .label(format!("Category: {}", selected.label()))
                .tooltip("Choose a configuration category")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.compact_categories_open = !this.compact_categories_open;
                    cx.notify();
                })),
        )
        .when(expanded, |panel| {
            panel
                .child(search_field(filter, &query, cx))
                .child(
                    div()
                        .id("settings-compact-scroll")
                        .max_h(rems(260. / 16.))
                        .overflow_y_scroll()
                        .child(navigation_items(selected, &query, cx)),
                )
        })
}

#[cfg(test)]
mod tests {
    use super::SettingsPage;

    #[test]
    fn categories_have_unique_ids_and_user_facing_labels() {
        let mut ids = std::collections::HashSet::new();

        for page in SettingsPage::ALL {
            assert!(ids.insert(page.id()), "duplicate settings page id");
            assert!(!page.label().is_empty(), "settings page without label");
            assert!(!page.description().is_empty(), "settings page without description");
        }
        assert_eq!(ids.len(), 9);
    }

    #[test]
    fn search_finds_options_by_category_and_setting_name() {
        assert!(SettingsPage::Appearance.matches_query("window transparency"));
        assert!(SettingsPage::Appearance.matches_query("THEME"));
        assert!(SettingsPage::Integrations.matches_query("claude bridge"));
        assert!(SettingsPage::Advanced.matches_query("install npm"));
        assert!(SettingsPage::Updates.matches_query("engram version"));
        assert!(SettingsPage::General.matches_query("setup"));
        assert!(!SettingsPage::Devices.matches_query("theme"));
        assert!(SettingsPage::ALL.iter().all(|page| page.matches_query("  ")));
    }
}
