//! Category navigation for Configuration. Existing sections keep their
//! state and persistence in SettingsView; navigation selects what to render.
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Icon, Sizable as _, v_flex};
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
    pub const ALL: [Self; 9] = [Self::General, Self::Appearance, Self::Agents, Self::Models, Self::Devices, Self::Integrations, Self::Notifications, Self::Updates, Self::Advanced];
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
}
pub(super) fn sidebar(selected: SettingsPage, cx: &mut Context<SettingsView>) -> Div {
    v_flex()
        .id("settings-navigation")
        .w(rems(200. / 16.))
        .flex_shrink_0()
        .min_h_0()
        .border_r_1()
        .border_color(ui::border())
        .child(
            div()
                .px_3()
                .pt_4()
                .pb_2()
                .text_xs()
                .text_color(ui::muted())
                .child("CATEGORIES"),
        )
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px_2()
                .gap_1()
                .children(SettingsPage::ALL.into_iter().map(|page| {
                    div()
                        .rounded_md()
                        .when(page == selected, |d| d.bg(ui::raised()))
                        .child(
                            Button::new(page.id())
                                .ghost()
                                .small()
                                .icon(Icon::new(page.icon()))
                                .label(page.label())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if this.page != page {
                                        this.page = page;
                                        cx.notify();
                                    }
                                })),
                        )
                })),
        )
}
