//! Optional vertical tab navigator. It shares the window's existing tabs,
//! panes and persisted workspaces; no second tab model is created.

use super::*;

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

    /// The dedicated left navigator is independent of the right Files sidebar.
    pub(super) fn render_vertical_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(rems(40. / 16.))
                    .flex_none()
                    .items_center()
                    .px_3()
                    .gap_2()
                    .child(Icon::new(IconName::List).xsmall().text_color(ui::faint()))
                    .child(ui::section_label("Tabs"))
                    .child(div().flex_1())
                    .child(
                        Button::new("tab-sidebar-horizontal")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::PanelLeftClose))
                            .tooltip("Move tabs back to the top bar")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_tab_layout(TabLayout::Horizontal, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_vertical_tabs(cx)),
            )
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
            .gap_0p5();
        for (index, space) in self.spaces.iter().enumerate() {
            let active_space = index == self.current;
            let name = space.name.clone();
            let count = space.tab_count();
            // Mirror the activity shown by the horizontal workspace switcher.
            let activity = self.space_activity(space);
            let status = if activity.needs_you > 0 {
                Some(("needs you", ui::warning()))
            } else if activity.working > 0 {
                Some(("working", ui::accent()))
            } else {
                None
            };
            list = list.child(
                h_flex()
                    .id(("vertical-space", index))
                    .mt_2()
                    .px_2()
                    .h(rems(27. / 16.))
                    .rounded_md()
                    .gap_2()
                    .cursor_pointer()
                    .when(active_space, |d| d.bg(ui::surface_hover()))
                    .hover(|d| d.bg(ui::raised()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.switch_workspace(index, window, cx)
                    }))
                    .child(Icon::new(IconName::Folder).xsmall().text_color(ui::muted()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .child(name),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_2()
                            .items_center()
                            .children(status.map(|(label, color)| {
                                h_flex()
                                    .gap_1()
                                    .items_center()
                                    .child(div().size(rems(6. / 16.)).rounded_full().bg(color))
                                    .child(div().text_xs().text_color(color).child(label))
                            }))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(ui::faint())
                                    .child(count.to_string()),
                            ),
                    ),
            );
            if !active_space {
                continue;
            }
            let mut shown = 0;
            let mut last_pinned = None;
            for (tab_index, tab) in space.tabs.iter().enumerate() {
                let title = self.pane_title(tab.focused);
                if !query.is_empty() && !title.to_lowercase().contains(&query) {
                    continue;
                }
                if last_pinned != Some(tab.pinned) {
                    last_pinned = Some(tab.pinned);
                    list = list.child(
                        div()
                            .px_2()
                            .pt_2()
                            .pb_1()
                            .text_xs()
                            .text_color(ui::faint())
                            .child(if tab.pinned { "Pinned" } else { "Open tabs" }),
                    );
                }
                shown += 1;
                list = list.child(self.render_vertical_tab(tab_index, tab, title, cx));
            }
            if shown == 0 {
                list = list.child(
                    div()
                        .py_4()
                        .text_center()
                        .text_xs()
                        .text_color(ui::muted())
                        .child(if query.is_empty() {
                            "No open tabs"
                        } else {
                            "No matching tabs"
                        }),
                );
            }
        }
        let canvas = cx.entity().downgrade();
        let tower = canvas.clone();
        v_flex()
            .size_full()
            .child(
                h_flex().p_2().child(
                    ui::field()
                        .flex_1()
                        .min_w_0()
                        .child(Icon::new(IconName::Search).xsmall().text_color(ui::faint()))
                        .child(Input::new(&self.tab_search).appearance(false).small()),
                ),
            )
            .child(list)
            .child(
                h_flex()
                    .flex_none()
                    .p_2()
                    .gap_1()
                    .child(
                        Button::new("vertical-new-agent")
                            .small()
                            .ghost()
                            .icon(Icon::new(IconName::Bot))
                            .label("Agent")
                            .tooltip(format!(
                                "New agent tab ({})",
                                ui::platform_shortcut("⌘T", "Ctrl+T")
                            ))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_new_tab(&NewTab, window, cx)
                            })),
                    )
                    .child(
                        Button::new("vertical-new-terminal")
                            .small()
                            .ghost()
                            .icon(Icon::new(IconName::SquareTerminal))
                            .label("Terminal")
                            .tooltip(format!(
                                "New terminal tab ({})",
                                ui::platform_shortcut("⌘⇧T", "Ctrl+Shift+T")
                            ))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_new_terminal_tab(&NewTerminalTab, window, cx)
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("vertical-more-tabs")
                            .small()
                            .ghost()
                            .icon(Icon::new(IconName::Plus))
                            .tooltip("New Canvas or Tower tab")
                            .dropdown_menu(move |menu, _, _| {
                                let canvas = canvas.clone();
                                let tower = tower.clone();
                                menu.item(
                                    PopupMenuItem::new("Canvas tab")
                                        .icon(Icon::new(IconName::PenTool))
                                        .on_click(move |_, window, cx| {
                                            let _ = canvas.update(cx, |this, cx| {
                                                this.on_new_canvas_tab(&NewCanvasTab, window, cx)
                                            });
                                        }),
                                )
                                .item(
                                    PopupMenuItem::new("Tower tab")
                                        .icon(Icon::new(IconName::Building2))
                                        .on_click(move |_, window, cx| {
                                            let _ = tower
                                                .update(cx, |this, cx| this.open_tower(window, cx));
                                        }),
                                )
                            })
                            .on_open_change(|open, window, cx| {
                                crate::features::browser::popover_open(
                                    "vertical-more-tabs",
                                    *open,
                                    window,
                                    cx,
                                )
                            }),
                    ),
            )
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
        let color = if needs_you {
            ui::warning()
        } else if running {
            ui::accent()
        } else {
            ui::faint()
        };
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
            .h(rems(36. / 16.))
            .px_2()
            .gap_2()
            .rounded_md()
            .cursor_pointer()
            .when(active, |d| d.bg(ui::raised()).text_color(ui::fg()))
            .when(!active, |d| {
                d.text_color(ui::muted())
                    .hover(|s| s.bg(ui::surface_hover()).text_color(ui::fg()))
            })
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
            .child(
                div()
                    .id(("vertical-tab-grip", id))
                    .flex_none()
                    .cursor_grab()
                    .on_drag(
                        TabDrag {
                            tab: id,
                            title: title.clone(),
                        },
                        |drag, _, _, cx| cx.new(|_| DragChip(drag.title.clone())),
                    )
                    .child(
                        Icon::new(IconName::GripHorizontal)
                            .xsmall()
                            .text_color(ui::faint()),
                    ),
            )
            .child(
                div()
                    .size(rems(17. / 16.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Icon::new(kind.icon()).xsmall().text_color(color)),
            )
            .child(
                match self.renaming.as_ref().filter(|r| r.pane == tab.focused) {
                    Some(rename) => div()
                        .flex_1()
                        .min_w_0()
                        .h(rems(24. / 16.))
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
                        .text_xs()
                        .tooltip({
                            let title = title.clone();
                            move |window, cx| Tooltip::new(title.clone()).build(window, cx)
                        })
                        .child(title)
                        .into_any_element(),
                },
            )
            .when(pinned, |d| {
                d.child(
                    Icon::new(IconName::Star)
                        .xsmall()
                        .text_color(ui::champagne()),
                )
            })
            .when(running || needs_you, |d| {
                d.child(div().size(rems(6. / 16.)).rounded_full().bg(color))
            })
            .child(
                div()
                    .id(("vertical-tab-actions-wrapper", id))
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
                                    .icon(Icon::new(IconName::Star))
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
