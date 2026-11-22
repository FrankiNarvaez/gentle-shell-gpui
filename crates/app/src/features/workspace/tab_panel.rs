//! Tab placement and the tab operations the vertical sidebar shares with the
//! horizontal strip. It shares the window's existing tabs, panes and
//! persisted workspaces; no second tab model is created. The sidebar itself
//! is drawn in `sidebars/`.

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

    /// The horizontal strip and the sidebar use the same placement menu.
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

    /// One of the sidebar footer's two new-tab tiles: a `surface` button the
    /// height of a field with its icon and label, lifting to `fg` on hover;
    /// the tooltip names the tab and its shortcut.
    pub(super) fn new_tab_tile(
        id: &'static str,
        icon: IconName,
        label: &'static str,
        keys: &'static str,
        tooltip: &'static str,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        let tooltip = SharedString::from(format!("{tooltip} ({keys})"));
        h_flex()
            .id(id)
            .flex_1()
            .min_w_0()
            .h(rems(34. / 16.))
            .px_2()
            .gap_2()
            .justify_center()
            .rounded_lg()
            .bg(ui::surface())
            .cursor_pointer()
            .text_size(rems(12.5 / 16.))
            .text_color(ui::muted())
            .hover(|s| s.bg(ui::surface_hover()).text_color(ui::fg()))
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .on_click(on_click)
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .child(Icon::new(icon).small())
            .child(div().min_w_0().truncate().child(label))
    }

    /// Pin, rename, move and close one tab of the shown workspace. Sidebar
    /// rows keep Move up / Move down; horizontal tabs use `compact` to omit
    /// them.
    pub(super) fn tab_actions_menu(
        menu: PopupMenu,
        weak: WeakEntity<Self>,
        tab: TabMenu,
    ) -> PopupMenu {
        let TabMenu {
            id,
            kind,
            pinned,
            can_up,
            can_down,
            compact,
        } = tab;
        let (pin, rename, up, down, close) =
            (weak.clone(), weak.clone(), weak.clone(), weak.clone(), weak);
        let mut menu = menu
            .item(
                PopupMenuItem::new(if pinned { "Unpin tab" } else { "Pin tab" })
                    .icon(Icon::new(if pinned {
                        IconName::PinOff
                    } else {
                        IconName::Pin
                    }))
                    .on_click(move |_, _, cx| {
                        let _ = pin.update(cx, |this, cx| this.toggle_tab_pin(id, cx));
                    }),
            )
            .item(
                PopupMenuItem::new("Rename")
                    .icon(Icon::new(IconName::Pencil))
                    .disabled(kind != PaneKind::Agent)
                    .on_click(move |_, window, cx| {
                        let _ = rename.update(cx, |this, cx| {
                            if let Some(i) = this.space().tabs.iter().position(|t| t.id == id) {
                                this.start_tab_rename(i, window, cx);
                            }
                        });
                    }),
            );
        if !compact {
            menu = menu
                .separator()
                .item(
                    PopupMenuItem::new("Move up")
                        .icon(Icon::new(IconName::ArrowUp))
                        .disabled(!can_up)
                        .on_click(move |_, _, cx| {
                            let _ = up.update(cx, |this, cx| this.move_tab_by(id, -1, cx));
                        }),
                )
                .item(
                    PopupMenuItem::new("Move down")
                        .icon(Icon::new(IconName::ArrowDown))
                        .disabled(!can_down)
                        .on_click(move |_, _, cx| {
                            let _ = down.update(cx, |this, cx| this.move_tab_by(id, 1, cx));
                        }),
                );
        }
        menu.separator().item(
            PopupMenuItem::new("Close tab")
                .icon(Icon::new(IconName::X))
                .on_click(move |_, window, cx| {
                    let _ = close.update(cx, |this, cx| {
                        if let Some(i) = this.space().tabs.iter().position(|t| t.id == id) {
                            this.close_tab(i, window, cx);
                        }
                    });
                }),
        )
    }

    /// Right-click on empty space of the tab area: create a workspace or
    /// switch between horizontal and vertical tabs.
    pub(super) fn empty_area_menu(
        menu: PopupMenu,
        weak: WeakEntity<Self>,
        layout: TabLayout,
    ) -> PopupMenu {
        let new_space = weak.clone();
        let menu = menu
            .item(
                PopupMenuItem::new("New workspace")
                    .icon(Icon::new(IconName::FolderPlus))
                    .on_click(move |_, window, cx| {
                        let _ = new_space.update(cx, |this, cx| {
                            this.pick_workspace_folder_for_menu(window, cx)
                        });
                    }),
            )
            .separator();
        Self::tab_placement_menu(menu, layout, weak)
    }
}

/// The tab a [`Workspace::tab_actions_menu`] acts on, and which moves it allows.
#[derive(Clone, Copy)]
pub(super) struct TabMenu {
    pub id: usize,
    pub kind: PaneKind,
    pub pinned: bool,
    pub can_up: bool,
    pub can_down: bool,
    /// Horizontal tabs omit Move up / Move down.
    pub compact: bool,
}
