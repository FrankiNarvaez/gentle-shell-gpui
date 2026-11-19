//! Optional vertical tab navigator. It shares the window's existing tabs,
//! panes and persisted workspaces; no second tab model is created.

use super::view::{new_tab_menu, workspace_avatar};
use super::*;

/// A navigator row, as tall as a Files tree row.
const TREE_ROW: f32 = 26.;
/// The tab rows' indent under their workspace: the chevron and its gap, so
/// a tab's icon lines up with the workspace's letter badge.
const TREE_INDENT: f32 = 18.;

impl Workspace {
    pub(super) fn toggle_tab_layout(&mut self, cx: &mut Context<Self>) {
        let next = match self.tab_layout {
            TabLayout::Horizontal => TabLayout::Vertical,
            TabLayout::Vertical => TabLayout::Horizontal,
        };
        self.set_tab_layout(next, cx);
    }

    /// All presentation controls share one idempotent, persisted state change.
    pub(super) fn set_tab_layout(&mut self, layout: TabLayout, cx: &mut Context<Self>) {
        if self.tab_layout == layout {
            return;
        }
        self.tab_layout = layout;
        // File browsing is a separate right sidebar; switching tab placement
        // must neither open nor hide that independent panel.
        self.schedule_save(cx);
        cx.notify();
    }

    /// The horizontal strip and vertical rows use the same placement menu.
    pub(super) fn tab_placement_menu(
        menu: PopupMenu,
        layout: TabLayout,
        workspace: WeakEntity<Self>,
    ) -> PopupMenu {
        let horizontal = workspace.clone();
        menu.item(
            PopupMenuItem::new("Horizontal tabs")
                .icon(Icon::new(IconName::List))
                .checked(layout == TabLayout::Horizontal)
                .on_click(move |_, _, cx| {
                    let _ = horizontal.update(cx, |this, cx| {
                        this.set_tab_layout(TabLayout::Horizontal, cx)
                    });
                }),
        )
        .item(
            PopupMenuItem::new("Vertical tabs")
                .icon(Icon::new(IconName::PanelLeft))
                .checked(layout == TabLayout::Vertical)
                .on_click(move |_, _, cx| {
                    let _ = workspace
                        .update(cx, |this, cx| this.set_tab_layout(TabLayout::Vertical, cx));
                }),
        )
    }

    /// Keep the selected tab stable when rows move, and pins ahead of other tabs.
    fn with_reordered_tabs(&mut self, change: impl FnOnce(&mut Vec<Tab>), cx: &mut Context<Self>) {
        let space = self.space_mut();
        let active = space.tabs.get(space.active).map(|tab| tab.id);
        change(&mut space.tabs);
        if let Some(active) = active {
            space.active = space
                .tabs
                .iter()
                .position(|tab| tab.id == active)
                .unwrap_or(0);
        }
        self.schedule_save(cx);
        cx.notify();
    }

    fn toggle_tab_pin(&mut self, id: usize, cx: &mut Context<Self>) {
        self.with_reordered_tabs(
            |tabs| {
                let Some(index) = tabs.iter().position(|tab| tab.id == id) else {
                    return;
                };
                let mut tab = tabs.remove(index);
                tab.pinned = !tab.pinned;
                let boundary = tabs.iter().take_while(|t| t.pinned).count();
                tabs.insert(boundary, tab);
            },
            cx,
        );
    }

    fn move_tab_by(&mut self, id: usize, direction: isize, cx: &mut Context<Self>) {
        self.with_reordered_tabs(
            |tabs| {
                let Some(index) = tabs.iter().position(|tab| tab.id == id) else {
                    return;
                };
                let Some(target) = index.checked_add_signed(direction) else {
                    return;
                };
                if target < tabs.len() && tabs[index].pinned == tabs[target].pinned {
                    tabs.swap(index, target);
                }
            },
            cx,
        );
    }

    /// Use the same tab-reordering operation as the horizontal strip.
    fn reorder_tab_at(&mut self, moving: usize, target: usize, cx: &mut Context<Self>) {
        if let Some(index) = self.space().tabs.iter().position(|tab| tab.id == target) {
            self.move_tab(moving, index, cx);
        }
    }

    /// The dedicated left navigator, shaped like the Files sidebar on the
    /// right: a 40 px header with its actions, a compact search field, then
    /// the workspaces as a tree with the current one's tabs under it.
    pub(super) fn render_vertical_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        let header = h_flex()
            .h(rems(40. / 16.))
            .flex_none()
            .pl_3()
            .pr_1p5()
            .gap_0p5()
            .child(Icon::new(IconName::List).small().text_color(ui::muted()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .ml_1()
                    .truncate()
                    .text_size(rems(13.5 / 16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ui::fg())
                    .child("Tabs"),
            )
            .child(
                Button::new("vertical-new-tab")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::Plus))
                    .tooltip("New tab")
                    .dropdown_menu(move |menu, window, _| new_tab_menu(menu, weak.clone(), window))
                    .on_open_change(|open, window, cx| {
                        crate::features::browser::popover_open(
                            "vertical-new-tab",
                            *open,
                            window,
                            cx,
                        )
                    }),
            )
            .child(
                Button::new("tab-sidebar-horizontal")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::PanelLeftClose))
                    .tooltip("Move tabs to the title bar")
                    .on_click(
                        cx.listener(|this, _, _, cx| {
                            this.set_tab_layout(TabLayout::Horizontal, cx)
                        }),
                    ),
            );
        let search = h_flex().flex_none().px_2().pb_1p5().child(
            ui::field()
                .flex_1()
                .min_w_0()
                .h(rems(30. / 16.))
                .child(Icon::new(IconName::Search).small().text_color(ui::faint()))
                .child(Input::new(&self.tab_search).appearance(false).small()),
        );
        v_flex()
            .size_full()
            .child(header)
            .child(search)
            .child(self.render_vertical_tabs(cx))
    }

    fn render_vertical_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.tab_search.read(cx).value().trim().to_lowercase();
        let mut list = v_flex()
            .id(("vertical-tabs-list", self.current))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_2()
            .pb_2()
            .gap_px();
        for (index, space) in self.spaces.iter().enumerate() {
            let current = index == self.current;
            // The workspace switcher's dot: amber when an agent needs you,
            // the accent while one works.
            let activity = self.space_activity(space);
            let dot = if activity.needs_you > 0 {
                Some((ui::warning(), "An agent here needs you"))
            } else if activity.working > 0 {
                Some((ui::accent(), "Agents are working here"))
            } else {
                None
            };
            list = list.child(
                h_flex()
                    .id(("vertical-space", index))
                    .flex_none()
                    .h(rems(TREE_ROW / 16.))
                    .pl(rems(4. / 16.))
                    .pr_2()
                    .gap_1p5()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|d| d.bg(ui::surface_hover()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.switch_workspace(index, window, cx)
                    }))
                    .child(
                        Icon::new(if current {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .xsmall()
                        .text_color(ui::faint()),
                    )
                    .child(workspace_avatar(&space.name))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(rems(12.5 / 16.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if current { ui::fg() } else { ui::muted() })
                            .child(space.name.clone()),
                    )
                    .when_some(dot, |d, (color, tip)| {
                        d.child(
                            div()
                                .id(("vertical-space-activity", index))
                                .size(rems(6. / 16.))
                                .flex_none()
                                .rounded_full()
                                .bg(color)
                                .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx)),
                        )
                    })
                    .child(
                        div()
                            .flex_none()
                            .font_family(ui::MONO)
                            .text_size(rems(10.5 / 16.))
                            .text_color(ui::faint())
                            .child(space.tab_count().to_string()),
                    ),
            );
            if !current {
                continue;
            }
            let mut shown = 0;
            for (tab_index, tab) in space.tabs.iter().enumerate() {
                let title = self.pane_title(tab.focused);
                if !query.is_empty() && !title.to_lowercase().contains(&query) {
                    continue;
                }
                shown += 1;
                list = list.child(self.render_vertical_tab(tab_index, tab, title, cx));
            }
            if shown == 0 {
                list = list.child(
                    div()
                        .h(rems(TREE_ROW / 16.))
                        .flex()
                        .items_center()
                        .pl(rems((4. + TREE_INDENT) / 16.))
                        .text_xs()
                        .text_color(ui::faint())
                        .child(if query.is_empty() {
                            "No open tabs"
                        } else {
                            "No matching tabs"
                        }),
                );
            }
        }
        list
    }

    fn render_vertical_tab(
        &self,
        index: usize,
        tab: &Tab,
        title: SharedString,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = tab.id;
        let active = self.space().active == index;
        let pinned = tab.pinned;
        let needs_you = tab
            .tree
            .leaves()
            .iter()
            .any(|pane| self.panes.get(pane).is_some_and(|pane| pane.needs_you));
        let running = self.tab_running(tab);
        // The kind icon carries the tab's state, like the horizontal strip.
        let tone = if needs_you {
            ui::warning()
        } else if running {
            ui::accent()
        } else if active {
            ui::muted()
        } else {
            ui::faint()
        };
        let group = SharedString::from(format!("vertical-tab-{id}"));
        let kind = self
            .panes
            .get(&tab.focused)
            .map(|p| p.item.kind)
            .unwrap_or(PaneKind::Agent);
        let weak = cx.entity().downgrade();
        let placement = weak.clone();
        let pin = weak.clone();
        let up = weak.clone();
        let down = weak.clone();
        let rename = weak.clone();
        let close = weak.clone();
        let total = self.space().tabs.len();
        let can_up = index > 0 && self.space().tabs[index - 1].pinned == pinned;
        let can_down = index + 1 < total && self.space().tabs[index + 1].pinned == pinned;
        h_flex()
            .id(("vertical-tab", id))
            .group(group.clone())
            .flex_none()
            .h(rems(TREE_ROW / 16.))
            .pl(rems(4. / 16.))
            .pr_1()
            .gap_1p5()
            .rounded_md()
            .cursor_pointer()
            .when(active, |d| d.bg(ui::raised()))
            .when(!active, |d| d.hover(|s| s.bg(ui::surface_hover())))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.click_count() >= 2 {
                    this.start_tab_rename(index, window, cx);
                } else {
                    this.activate(index, window, cx);
                }
            }))
            .drag_over::<TabDrag>(|style, _, _, _| style.bg(ui::petal().opacity(0.18)))
            .on_drop(
                cx.listener(move |this, drag: &TabDrag, _, cx| {
                    this.reorder_tab_at(drag.tab, id, cx)
                }),
            )
            .context_menu(move |menu, _, _| {
                Self::tab_placement_menu(menu, TabLayout::Vertical, placement.clone())
            })
            // The grip only shows on hover, in the tree's indent.
            .child(
                div()
                    .id(("vertical-tab-grip", id))
                    .flex_none()
                    .w(rems(TREE_INDENT / 16.))
                    .flex()
                    .justify_center()
                    .cursor_grab()
                    .invisible()
                    .group_hover(group.clone(), |s| s.visible())
                    .on_drag(
                        TabDrag {
                            tab: id,
                            title: title.clone(),
                        },
                        |drag, _, _, cx| cx.new(|_| DragChip(drag.title.clone())),
                    )
                    .child(
                        Icon::new(IconName::GripVertical)
                            .xsmall()
                            .text_color(ui::faint()),
                    ),
            )
            .child(Icon::new(kind.icon()).small().text_color(tone))
            .child(
                match self.renaming.as_ref().filter(|r| r.pane == tab.focused) {
                    Some(rename) => div()
                        .flex_1()
                        .min_w_0()
                        .h(rems(20. / 16.))
                        .px_1()
                        .rounded_md()
                        .bg(ui::well())
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_action(cx.listener(Self::cancel_tab_rename))
                        .child(Input::new(&rename.input).appearance(false).xsmall())
                        .into_any_element(),
                    None => div()
                        .id(("vertical-tab-title", id))
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .text_size(rems(12.5 / 16.))
                        .text_color(if active { ui::fg() } else { ui::muted() })
                        .tooltip({
                            let title = title.clone();
                            move |window, cx| Tooltip::new(title.clone()).build(window, cx)
                        })
                        .child(title)
                        .into_any_element(),
                },
            )
            .when(pinned, |d| {
                d.child(Icon::new(IconName::Pin).xsmall().text_color(ui::faint()))
            })
            .when(running || needs_you, |d| {
                d.child(
                    div()
                        .size(rems(6. / 16.))
                        .flex_none()
                        .rounded_full()
                        .bg(tone),
                )
            })
            .child(
                div()
                    .id(("vertical-tab-actions-wrapper", id))
                    .flex_none()
                    .when(!active, |d| {
                        d.invisible().group_hover(group.clone(), |s| s.visible())
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        Button::new(("vertical-tab-actions", id))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::Ellipsis))
                            .tooltip("Tab actions")
                            .dropdown_menu(move |menu, _, _| {
                                menu.item(
                                    PopupMenuItem::new(if pinned {
                                        "Unpin tab"
                                    } else {
                                        "Pin tab"
                                    })
                                    .icon(Icon::new(if pinned {
                                        IconName::PinOff
                                    } else {
                                        IconName::Pin
                                    }))
                                    .on_click({
                                        let pin = pin.clone();
                                        move |_, _, cx| {
                                            let _ = pin
                                                .update(cx, |this, cx| this.toggle_tab_pin(id, cx));
                                        }
                                    }),
                                )
                                .item(
                                    PopupMenuItem::new("Rename")
                                        .icon(Icon::new(IconName::Pencil))
                                        .disabled(kind != PaneKind::Agent)
                                        .on_click({
                                            let rename = rename.clone();
                                            move |_, window, cx| {
                                                let _ = rename.update(cx, |this, cx| {
                                                    if let Some(i) = this
                                                        .space()
                                                        .tabs
                                                        .iter()
                                                        .position(|t| t.id == id)
                                                    {
                                                        this.start_tab_rename(i, window, cx);
                                                    }
                                                });
                                            }
                                        }),
                                )
                                .separator()
                                .item(
                                    PopupMenuItem::new("Move up")
                                        .icon(Icon::new(IconName::ArrowUp))
                                        .disabled(!can_up)
                                        .on_click({
                                            let up = up.clone();
                                            move |_, _, cx| {
                                                let _ = up.update(cx, |this, cx| {
                                                    this.move_tab_by(id, -1, cx)
                                                });
                                            }
                                        }),
                                )
                                .item(
                                    PopupMenuItem::new("Move down")
                                        .icon(Icon::new(IconName::ArrowDown))
                                        .disabled(!can_down)
                                        .on_click({
                                            let down = down.clone();
                                            move |_, _, cx| {
                                                let _ = down.update(cx, |this, cx| {
                                                    this.move_tab_by(id, 1, cx)
                                                });
                                            }
                                        }),
                                )
                                .separator()
                                .item(
                                    PopupMenuItem::new("Close tab")
                                        .icon(Icon::new(IconName::X))
                                        .on_click({
                                            let close = close.clone();
                                            move |_, window, cx| {
                                                let _ = close.update(cx, |this, cx| {
                                                    if let Some(i) = this
                                                        .space()
                                                        .tabs
                                                        .iter()
                                                        .position(|t| t.id == id)
                                                    {
                                                        this.close_tab(i, window, cx);
                                                    }
                                                });
                                            }
                                        }),
                                )
                            }),
                    ),
            )
    }
}
