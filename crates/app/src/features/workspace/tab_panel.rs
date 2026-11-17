//! Optional vertical tab navigator. It shares the window's existing tabs,
//! panes and persisted workspaces; no second tab model is created.

use super::*;
use gpui_kit::component::Selectable as _;

impl Workspace {
    pub(super) fn toggle_tab_layout(&mut self, cx: &mut Context<Self>) {
        self.tab_layout = match self.tab_layout {
            TabLayout::Horizontal => TabLayout::Vertical,
            TabLayout::Vertical => TabLayout::Horizontal,
        };
        if self.tab_layout == TabLayout::Vertical {
            self.sidebar_section = SidebarSection::Tabs;
            if !self.sidebar_visible {
                self.sidebar_visible = true;
                cx.emit(WorkspaceEvent::SidebarToggled(true));
            }
        }
        cx.emit(WorkspaceEvent::FilesVisibilityChanged(self.files_sidebar_visible()));
        self.schedule_save(cx);
        cx.notify();
    }

    fn select_sidebar_section(&mut self, section: SidebarSection, cx: &mut Context<Self>) {
        self.sidebar_section = section;
        cx.emit(WorkspaceEvent::FilesVisibilityChanged(self.files_sidebar_visible()));
        cx.notify();
    }

    /// Keep the selected tab stable when rows move, and pins ahead of other tabs.
    fn with_reordered_tabs(
        &mut self,
        change: impl FnOnce(&mut Vec<Tab>),
        cx: &mut Context<Self>,
    ) {
        let space = self.space_mut();
        let active = space.tabs.get(space.active).map(|tab| tab.id);
        change(&mut space.tabs);
        if let Some(active) = active {
            space.active = space.tabs.iter().position(|tab| tab.id == active).unwrap_or(0);
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

    /// Called by the drag-and-drop targets in the sidebar.
    fn reorder_tab_at(&mut self, moving: usize, target: usize, cx: &mut Context<Self>) {
        if moving == target {
            return;
        }
        self.with_reordered_tabs(
            |tabs| {
                let Some(from) = tabs.iter().position(|tab| tab.id == moving) else {
                    return;
                };
                let Some(to) = tabs.iter().position(|tab| tab.id == target) else {
                    return;
                };
                if tabs[from].pinned != tabs[to].pinned {
                    return;
                }
                let tab = tabs.remove(from);
                tabs.insert(to, tab);
            },
            cx,
        );
    }

    /// Tabs and the existing Files/Changes/Branches view share one sidebar slot.
    pub(super) fn render_vertical_sidebar(
        &self,
        files: Option<AnyView>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let tabs_active = self.sidebar_section == SidebarSection::Tabs || files.is_none();
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(rems(40. / 16.))
                    .flex_none()
                    .px_2()
                    .gap_1()
                    .child(
                        Button::new("tab-sidebar-tabs")
                            .small()
                            .ghost()
                            .selected(tabs_active)
                            .label("Tabs")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.select_sidebar_section(SidebarSection::Tabs, cx)
                            })),
                    )
                    .when(files.is_some(), |row| {
                        row.child(
                            Button::new("tab-sidebar-files")
                                .small()
                                .ghost()
                                .selected(!tabs_active)
                                .label("Files")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.select_sidebar_section(SidebarSection::Files, cx)
                                })),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        Button::new("tab-sidebar-horizontal")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::PanelLeftClose))
                            .tooltip("Return to horizontal tabs")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_tab_layout(cx)
                            })),
                    ),
            )
            .child(
                div().flex_1().min_h_0().child(if tabs_active {
                    self.render_vertical_tabs(cx).into_any_element()
                } else {
                    files
                        .map(|view| {
                            view.cached(StyleRefinement::default().size_full())
                                .into_any_element()
                        })
                        .unwrap_or_else(|| div().into_any_element())
                }),
            )
    }

    fn render_vertical_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.tab_search.read(cx).value().trim().to_lowercase();
        let mut list = v_flex()
            .id("vertical-tabs-list")
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
                        div()
                            .text_xs()
                            .text_color(ui::faint())
                            .child(count.to_string()),
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
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .p_2()
                    .gap_1()
                    .child(
                        ui::field()
                            .flex_1()
                            .min_w_0()
                            .child(Icon::new(IconName::Search).xsmall().text_color(ui::faint()))
                            .child(Input::new(&self.tab_search).appearance(false).small()),
                    )
                    .child(
                        Button::new("vertical-new-agent")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::Plus))
                            .tooltip("New agent tab")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_new_tab(&NewTab, window, cx)
                            })),
                    ),
            )
            .child(list)
            .child(
                h_flex()
                    .flex_none()
                    .p_2()
                    .gap_2()
                    .child(
                        Button::new("vertical-new-terminal")
                            .small()
                            .ghost()
                            .icon(Icon::new(IconName::SquareTerminal))
                            .label("Terminal")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_new_terminal_tab(&NewTerminalTab, window, cx)
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_xs()
                            .text_color(ui::faint())
                            .child("Drag to reorder"),
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
        let needs_you = tab.tree.leaves().iter().any(|pane| {
            self.panes.get(pane).is_some_and(|pane| pane.needs_you)
        });
        let running = self.tab_running(tab);
        let color = if needs_you {
            ui::warning()
        } else if running {
            ui::accent()
        } else {
            ui::faint()
        };
        let kind = self.panes.get(&tab.focused)
            .map(|p| p.item.kind)
            .unwrap_or(PaneKind::Agent);
        let weak = cx.entity().downgrade();
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
            .on_drag(
                TabDrag { tab: id, title: title.clone() },
                |drag, _, _, cx| cx.new(|_| DragChip(drag.title.clone())),
            )
            .on_drop(cx.listener(move |this, drag: &TabDrag, _, cx| {
                this.reorder_tab_at(drag.tab, id, cx)
            }))
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
                d.child(Icon::new(IconName::Star).xsmall().text_color(ui::champagne()))
            })
            .when(running || needs_you, |d| {
                d.child(div().size(rems(6. / 16.)).rounded_full().bg(color))
            })
            .child(
                Button::new(("vertical-tab-actions", id))
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::Ellipsis))
                    .tooltip("Tab actions")
                    .dropdown_menu(move |menu, _, _| {
                        menu.item(
                            PopupMenuItem::new(if pinned { "Unpin tab" } else { "Pin tab" })
                                .icon(Icon::new(IconName::Star))
                                .on_click({
                                    let pin = pin.clone();
                                    move |_, _, cx| {
                                        let _ = pin.update(cx, |this, cx| this.toggle_tab_pin(id, cx));
                                    }
                                }),
                        )
                        .item(
                            PopupMenuItem::new("Rename")
                                .icon(Icon::new(IconName::Pencil))
                                .on_click({
                                    let rename = rename.clone();
                                    move |_, window, cx| {
                                        let _ = rename.update(cx, |this, cx| {
                                            if let Some(i) = this.space().tabs.iter().position(|t| t.id == id) {
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
                                        let _ = up.update(cx, |this, cx| this.move_tab_by(id, -1, cx));
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
                                        let _ = down.update(cx, |this, cx| this.move_tab_by(id, 1, cx));
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
                                            if let Some(i) = this.space().tabs.iter().position(|t| t.id == id) {
                                                this.close_tab(i, window, cx);
                                            }
                                        });
                                    }
                                }),
                        )
                    }),
            )
    }
}
