//! The sidebar's pieces, measured like the Files sidebar: a 40 px header,
//! the compact search field, 26 px rows, section labels, and the Agent /
//! Terminal tiles at the bottom.

use super::super::badge::workspace_avatar;
use super::*;

/// A row, as tall as a Files tree row.
pub(super) const ROW: f32 = 26.;
/// A tab row's indent under its workspace row: the chevron and its gap, so
/// the tab's icon lines up with the workspace's badge.
pub(super) const INDENT: f32 = 22.;

impl Workspace {
    /// The header: the shown workspace (a click switches, a right-click
    /// edits its badge), then `actions`.
    pub(super) fn sidebar_header(&self, actions: Vec<AnyElement>, cx: &mut Context<Self>) -> Div {
        let space = self.space();
        let mark = Mark::of(space);
        h_flex()
            .h(rems(40. / 16.))
            .flex_none()
            .pl_3()
            .pr_1p5()
            .gap_0p5()
            .child(
                h_flex()
                    .id("sidebar-workspace")
                    .flex_1()
                    .min_w_0()
                    .h(rems(ROW / 16.))
                    .ml_neg_1()
                    .px_1()
                    .gap_1p5()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|s| s.bg(ui::surface_hover().opacity(0.6)))
                    .tooltip(|window, cx| {
                        Tooltip::new("Switch workspace · right-click for its badge")
                            .build(window, cx)
                    })
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_workspace_switcher(window, cx)),
                    )
                    .map(|d| self.with_badge_picker(d, self.current, cx))
                    .child(workspace_avatar(&mark))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(rems(13.5 / 16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ui::fg())
                            .child(space.name.clone()),
                    )
                    .child(
                        Icon::new(IconName::ChevronsUpDown)
                            .xsmall()
                            .text_color(ui::faint()),
                    ),
            )
            .children(actions)
    }

    /// New tab, and the way back to the title bar.
    pub(super) fn header_actions(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        vec![
            self.new_tab_button("sidebar-new-tab", None, cx),
            Button::new("sidebar-horizontal")
                .ghost()
                .xsmall()
                .icon(Icon::new(IconName::PanelLeftClose))
                .tooltip("Move tabs to the title bar")
                .on_click(
                    cx.listener(|this, _, _, cx| this.set_tab_layout(TabLayout::Horizontal, cx)),
                )
                .into_any_element(),
        ]
    }

    /// The search field; it filters the tabs of every workspace.
    pub(super) fn sidebar_search(&self) -> Div {
        h_flex().flex_none().px_2().pb_1p5().child(
            ui::field()
                .flex_1()
                .min_w_0()
                .h(rems(30. / 16.))
                .child(Icon::new(IconName::Search).small().text_color(ui::faint()))
                .child(Input::new(&self.tab_search).appearance(false).small()),
        )
    }

    /// Workspace `index`'s tabs that match the search.
    pub(super) fn visible_tabs(&self, index: usize, cx: &App) -> Vec<TabEntry> {
        let query = self.tab_search.read(cx).value().trim().to_lowercase();
        let mut tabs = self.sidebar_tabs(index);
        if !query.is_empty() {
            tabs.retain(|tab| tab.title.to_lowercase().contains(&query));
        }
        tabs
    }

    pub(super) fn searching(&self, cx: &App) -> bool {
        !self.tab_search.read(cx).value().trim().is_empty()
    }

    /// A group's label, as the Files and Settings sidebars set them.
    pub(super) fn section_row(label: &'static str) -> Div {
        div().px_2().pt_3().pb_1().child(ui::section_label(label))
    }

    /// The row a search with no match leaves, at `indent`.
    pub(super) fn quiet_row(text: &'static str, indent: f32) -> Div {
        div()
            .flex_none()
            .h(rems(ROW / 16.))
            .flex()
            .items_center()
            .pl(rems((6. + indent) / 16.))
            .text_xs()
            .text_color(ui::faint())
            .child(text)
    }

    /// A tab as a 26 px row: kind icon in its state's tone, title, pin,
    /// status dot and pane count; the shown tab raised, with the accent
    /// marker on its left.
    pub(super) fn tab_row(
        &self,
        tab: &TabEntry,
        indent: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tip = tab.tooltip();
        let row = h_flex()
            .id(SharedString::from(format!(
                "sidebar-tab-{}-{}",
                tab.space, tab.index
            )))
            .relative()
            .flex_none()
            .w_full()
            .h(rems(ROW / 16.))
            .pl(rems((6. + indent) / 16.))
            .pr_2()
            .gap_1p5()
            .rounded_md()
            .when(tab.shown, |d| d.bg(ui::raised()))
            .when(!tab.shown, |d| d.hover(|s| s.bg(ui::surface_hover())))
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .when(tab.shown, |d| {
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .top(rems(6. / 16.))
                        .bottom(rems(6. / 16.))
                        .w(px(2.))
                        .rounded_full()
                        .bg(ui::petal()),
                )
            })
            .child(Self::tab_icon(tab))
            .child(self.tab_title(
                tab,
                12.5,
                if tab.shown { ui::fg() } else { ui::muted() },
                cx,
            ))
            .when(tab.pinned, |d| {
                d.child(Icon::new(IconName::Pin).xsmall().text_color(ui::faint()))
            })
            .children(tab.status_dot())
            .when(tab.panes > 1, |d| {
                d.child(
                    div()
                        .flex_none()
                        .font_family(ui::mono())
                        .text_size(ui::code_rems(10.5))
                        .text_color(ui::faint())
                        .child(tab.panes.to_string()),
                )
            });
        self.tab_row_behavior(row, tab, cx)
    }

    /// A workspace as a 26 px row: `chevron` (open or closed) when it
    /// folds, its badge, name, activity dot and tab count. A right-click
    /// edits its badge; the caller adds what a click does.
    pub(super) fn space_row(
        &self,
        space: &SpaceEntry,
        chevron: Option<bool>,
        id: &'static str,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let tip = space.tooltip();
        h_flex()
            .id((id, space.index))
            .relative()
            .flex_none()
            .w_full()
            .h(rems(ROW / 16.))
            .pl(rems(4. / 16.))
            .pr_2()
            .gap_1p5()
            .rounded_md()
            .cursor_pointer()
            .hover(|d| d.bg(ui::surface_hover()))
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .map(|d| self.with_badge_picker(d, space.index, cx))
            .when_some(chevron, |d, open| {
                d.child(
                    Icon::new(if open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .xsmall()
                    .text_color(ui::faint()),
                )
            })
            .child(workspace_avatar(&space.mark))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(rems(12.5 / 16.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if space.current { ui::fg() } else { ui::muted() })
                    .child(space.name.clone()),
            )
            .when_some(space.dot(), |d, (color, _)| {
                d.child(
                    div()
                        .size(rems(6. / 16.))
                        .flex_none()
                        .rounded_full()
                        .bg(color),
                )
            })
            .child(
                div()
                    .flex_none()
                    .font_family(ui::mono())
                    .text_size(ui::code_rems(10.5))
                    .text_color(ui::faint())
                    .child(space.tabs.to_string()),
            )
    }
}
