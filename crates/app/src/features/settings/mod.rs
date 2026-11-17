//! Configuration: everything the user's Gentle Shell is already set up with,
//! read from the same homes the launcher uses, plus package management
//! (`gentle-shell install|remove`) and the Claude bridge's plan.

use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::slider::{SliderEvent, SliderState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::{features::app_update, shared::themes, shared::ui};

mod app;
mod background;
mod claude;
mod devices;
mod hydration;
mod layout;
mod maestro;
mod models;
mod navigation;
mod music;
mod notifications;
mod packages;
mod pages;
mod services;
mod snapshot;
mod theme;
mod upgrade;

pub use claude::forget_claude_auth;
pub use snapshot::npm_name;

use navigation::SettingsPage;
use snapshot::Snapshot;

pub enum SettingsEvent {
    Close,
    /// Packages or sign-ins changed: tabs should reload their backend.
    Changed,
    /// Run this Gentle Shell slash command in the active agent pane and
    /// report back through [`SettingsView::shell_done`].
    ShellCommand(String),
    /// `gentle-shell upgrade` finished: agent tabs restart on the new version.
    BackendUpdated,
    /// gentle-stream was installed or removed: the title bar's ♪ follows.
    MusicChanged,
    /// Configuration → Devices' switches changed.
    DevicesChanged(crate::features::setup::devices::Prefs),
    /// An action from the Setup & requirements checklist (sign in, a
    /// terminal command, an install that finished) for the window to carry out.
    Setup(crate::features::onboarding::OnboardingEvent),
}

pub struct SettingsView {
    cwd: PathBuf,
    page: SettingsPage,
    body_scroll: ScrollHandle,
    settings_search: Entity<InputState>,
    _settings_search: Subscription,
    compact_categories_open: bool,
    snapshot: Option<Snapshot>,
    /// Package input, progress, output and event subscription.
    package: packages::PackageUiState,
    _load: Task<()>,
    /// Existing shared lifecycle for package and Gentle Shell upgrade commands.
    _job: Task<()>,
    /// Window transparency, 0–100 %.
    transparency: Entity<SliderState>,
    _transparency: Subscription,
    /// Pending background-helper policy command and its feedback.
    background: background::BackgroundUiState,
    /// Gentle Shell upgrade status and its feedback.
    shell_update: upgrade::ShellUpdateUiState,
    /// Re-render as this app's own update moves along.
    _app_update: Subscription,
    /// Notification configuration write feedback.
    notifications: notifications::NotificationUiState,
    /// Setup & requirements: the onboarding checklist, embedded.
    setup: Entity<crate::features::onboarding::OnboardingView>,
    _setup: Subscription,
    /// UI testing (Maestro): what was found, the job running and its log.
    maestro: maestro::Maestro,
    /// Music (gentle-stream): what was found, the job running and its log.
    music: music::Music,
    /// Devices: the switches, what this machine has, an install running.
    devices: devices::Devices,
    /// Engram's own update.
    engram: upgrade::EngramUpdate,
    /// Theme: importing and sharing themes.
    theme_share: theme::ThemeShare,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

impl SettingsView {
    pub fn new(cwd: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let package = packages::PackageUiState::new(window, cx);
        let settings_search = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Find a setting…")
        });
        let settings_search_subscription =
            cx.subscribe(&settings_search, |this, input, event: &InputEvent, cx| {
                match event {
                    InputEvent::Change => cx.notify(),
                    InputEvent::PressEnter { .. } => {
                        let query = input.read(cx).value().to_string();
                        if !query.trim().is_empty()
                            && let Some(page) = SettingsPage::ALL
                                .into_iter()
                                .find(|page| page.matches_query(&query))
                        {
                            this.navigate_to(page, cx);
                        }
                    }
                    _ => {}
                }
            });
        let percent = (cx.global::<themes::Themes>().transparency * 100.).round();
        let transparency = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(100.)
                .step(1.)
                .default_value(percent)
        });
        // Drag re-tints live; the file is written once, on release.
        let transparency_sub = cx.subscribe(&transparency, |_, _, event: &SliderEvent, cx| {
            let (value, persist) = match event {
                SliderEvent::Change(value) => (value.start(), false),
                SliderEvent::Release(value) => (value.start(), true),
            };
            themes::set_transparency(value / 100., persist, cx);
        });
        let setup = cx.new(crate::features::onboarding::OnboardingView::embedded);
        let setup_sub = cx.subscribe(&setup, |_, _, event, cx| {
            cx.emit(SettingsEvent::Setup(event.clone()));
        });
        let mut this = Self {
            cwd,
            page: SettingsPage::default(),
            body_scroll: ScrollHandle::new(),
            settings_search,
            _settings_search: settings_search_subscription,
            compact_categories_open: false,
            snapshot: None,
            package,
            _load: Task::ready(()),
            _job: Task::ready(()),
            transparency,
            _transparency: transparency_sub,
            background: background::BackgroundUiState::default(),
            shell_update: upgrade::ShellUpdateUiState::default(),
            _app_update: cx.observe_global::<app_update::AppUpdate>(|_, cx| cx.notify()),
            notifications: notifications::NotificationUiState::default(),
            setup,
            _setup: setup_sub,
            maestro: maestro::Maestro::default(),
            music: music::Music::default(),
            devices: devices::Devices::default(),
            engram: upgrade::EngramUpdate::default(),
            theme_share: theme::ThemeShare::new(window, cx),
        };
        this.reload(cx);
        this.refresh_maestro(cx);
        this.refresh_music(cx);
        this.refresh_devices(cx);
        this
    }

    /// gentle-shell can start, as of the Setup & requirements checklist.
    pub fn setup_usable(&self, cx: &App) -> bool {
        self.setup.read(cx).usable()
    }

    /// Reuse a single navigation path for the sidebar, compact picker and search.
    fn navigate_to(&mut self, page: SettingsPage, cx: &mut Context<Self>) {
        self.compact_categories_open = false;
        if self.page != page {
            self.page = page;
            self.body_scroll.scroll_to_item(0);
        }
        cx.notify();
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let cwd = self.cwd.clone();
        self._load = cx.spawn(async move |this, cx| {
            let snapshot = cx
                .background_executor()
                .spawn(async move { services::snapshot::load(&cwd) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.snapshot = Some(snapshot);
                cx.notify();
            });
        });
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Keep settings usable when there is not enough horizontal space
        // for both a category rail and the existing settings controls.
        let compact = window.viewport_size().width < rems(820. / 16.).to_pixels(window.rem_size());
        let page = self.page;
        let body: AnyElement = match &self.snapshot {
            Some(snapshot) => self.render_page(page, snapshot, cx),
            None => h_flex()
                .p_6()
                .gap_2()
                .justify_center()
                .text_sm()
                .text_color(ui::muted())
                .child(Spinner::new().small().color(ui::accent()))
                .child("Reading your Gentle Shell configuration…")
                .into_any_element(),
        };
        let (backdrop, panel) = ui::screen(Some(960.));
        backdrop.occlude().child(ui::pop_in(
            "settings-screen",
            panel
                .w_full()
                .max_w(rems(960. / 16.))
                // A definite, viewport-clamped height is required for nested
                // flex children to shrink and for settings-body to scroll.
                // ui::screen already applies max_h_full() to the panel.
                .h(rems(720. / 16.))
                .child(
                    ui::screen_header()
                        .flex_shrink_0()
                        .child(
                            Icon::new(IconName::Settings)
                                .small()
                                .text_color(ui::muted()),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_base()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ui::fg())
                                .child("Configuration"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(ui::muted())
                                .child("Manage Gentle Shell and the desktop experience"),
                        )
                        .child(
                            Button::new("settings-refresh")
                                .ghost()
                                .xsmall()
                                .icon(Icon::new(IconName::RefreshCw))
                                .tooltip("Reload")
                                .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                        )
                        .child(
                            Button::new("settings-close")
                                .ghost()
                                .xsmall()
                                .icon(Icon::new(IconName::X))
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Close))),
                        ),
                )
                .child(
                    h_flex()
                        // h_flex defaults to items_center. That lets the
                        // settings column grow taller than this row and clips
                        // its scrollable body instead of constraining it.
                        .items_stretch()
                        .flex_1()
                        .min_h_0()
                        .when(!compact, |container| {
                            container.child(navigation::sidebar(page, &self.settings_search, cx))
                        })
                        .child(
                            v_flex()
                                // Fill the row's bounded height, not the
                                // intrinsic height of every settings card.
                                .h_full()
                                .flex_1()
                                .min_w_0()
                                .min_h_0()
                                .when(compact, |content| {
                                    content.child(navigation::compact(
                                        page,
                                        &self.settings_search,
                                        self.compact_categories_open,
                                        cx,
                                    ))
                                })
                                .child(
                                    v_flex()
                                        .flex_shrink_0()
                                        .px_4()
                                        .py_3()
                                        .gap_1()
                                        .border_b_1()
                                        .border_color(ui::border())
                                        .child(
                                            div()
                                                .text_base()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(ui::fg())
                                                .child(page.label()),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(ui::muted())
                                                .child(page.description()),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("settings-body")
                                        .flex_1()
                                        .min_h_0()
                                        .overflow_y_scroll()
                                        .track_scroll(&self.body_scroll)
                                        .child(body),
                                ),
                        ),
                ),
        ))
    }
}
