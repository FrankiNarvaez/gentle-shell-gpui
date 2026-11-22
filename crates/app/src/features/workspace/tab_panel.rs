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
        self.cancel_workspace_rename_state(cx);
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

    /// Keep the selected tab stable when rows move, and pins ahead of other
    /// tabs. Works on any workspace's tabs, built or not.
    fn with_reordered_tabs(
        &mut self,
        space: usize,
        change: impl FnOnce(&mut Vec<Tab>),
        cx: &mut Context<Self>,
    ) {
        let Some(space) = self.spaces.get_mut(space) else {
            return;
        };
        reorder_keeping_active(space, change);
        self.schedule_save(cx);
        cx.notify();
    }

    fn toggle_tab_pin(&mut self, space: usize, id: usize, cx: &mut Context<Self>) {
        self.with_reordered_tabs(space, |tabs| toggle_pin(tabs, id), cx);
    }

    fn move_tab_by(&mut self, space: usize, id: usize, direction: isize, cx: &mut Context<Self>) {
        self.with_reordered_tabs(space, |tabs| move_by(tabs, id, direction), cx);
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
            space,
            id,
            can_rename,
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
                        let _ = pin.update(cx, |this, cx| this.toggle_tab_pin(space, id, cx));
                    }),
            )
            .item(
                PopupMenuItem::new("Rename")
                    .icon(Icon::new(IconName::Pencil))
                    // Renaming needs a live session, which an unbuilt tab
                    // doesn't have (and building it just to rename isn't worth it).
                    .disabled(!can_rename)
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
                            let _ = up.update(cx, |this, cx| this.move_tab_by(space, id, -1, cx));
                        }),
                )
                .item(
                    PopupMenuItem::new("Move down")
                        .icon(Icon::new(IconName::ArrowDown))
                        .disabled(!can_down)
                        .on_click(move |_, _, cx| {
                            let _ = down.update(cx, |this, cx| this.move_tab_by(space, id, 1, cx));
                        }),
                );
        }
        menu.separator().item(
            PopupMenuItem::new("Close tab")
                .icon(Icon::new(IconName::X))
                .on_click(move |_, window, cx| {
                    let _ = close.update(cx, |this, cx| {
                        let index = this
                            .spaces
                            .get(space)
                            .and_then(|s| s.tabs.iter().position(|t| t.id == id));
                        if let Some(i) = index {
                            this.close_tab_in_space(space, i, window, cx);
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
                        let _ = new_space
                            .update(cx, |this, cx| this.pick_workspace_folder(None, window, cx));
                    }),
            )
            .separator();
        Self::tab_placement_menu(menu, layout, weak)
    }
}

/// The tab a [`Workspace::tab_actions_menu`] acts on, and which moves it allows.
#[derive(Clone, Copy)]
pub(super) struct TabMenu {
    /// The workspace the tab belongs to (not always the shown one).
    pub space: usize,
    pub id: usize,
    /// Rename needs a live agent session: the tab is built, an agent, and in
    /// the shown workspace.
    pub can_rename: bool,
    pub pinned: bool,
    pub can_up: bool,
    pub can_down: bool,
    /// Horizontal tabs omit Move up / Move down.
    pub compact: bool,
}

/// Apply `change` to the space's tabs; the active tab stays the same tab.
fn reorder_keeping_active(space: &mut Space, change: impl FnOnce(&mut Vec<Tab>)) {
    let active = space.tabs.get(space.active).map(|tab| tab.id);
    change(&mut space.tabs);
    if let Some(active) = active {
        space.active = space
            .tabs
            .iter()
            .position(|tab| tab.id == active)
            .unwrap_or(0);
    }
}

/// Flip tab `id`'s pin; it goes to the end of the pinned group, or, when
/// unpinned, to the start of the rest.
fn toggle_pin(tabs: &mut Vec<Tab>, id: usize) {
    let Some(index) = tabs.iter().position(|tab| tab.id == id) else {
        return;
    };
    let mut tab = tabs.remove(index);
    tab.pinned = !tab.pinned;
    let boundary = tabs.iter().take_while(|t| t.pinned).count();
    tabs.insert(boundary, tab);
}

/// Swap tab `id` with its neighbour in `direction`, never across the pinned
/// boundary.
fn move_by(tabs: &mut [Tab], id: usize, direction: isize) {
    let Some(index) = tabs.iter().position(|tab| tab.id == id) else {
        return;
    };
    let Some(target) = index.checked_add_signed(direction) else {
        return;
    };
    if target < tabs.len() && tabs[index].pinned == tabs[target].pinned {
        tabs.swap(index, target);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Space, Tab, TabState, move_by, reorder_keeping_active, toggle_pin};
    use crate::features::workspaces::{SavedPane, SavedTab};
    use crate::shared::pane::{PaneKind, PaneTree};

    fn unbuilt(id: usize, pinned: bool) -> Tab {
        Tab {
            id,
            pinned,
            state: TabState::Unbuilt(SavedTab {
                layout: PaneTree::Leaf(SavedPane {
                    kind: PaneKind::Terminal,
                    cwd: PathBuf::from("/x"),
                    path: None,
                    session: None,
                }),
                pinned,
                focused: 0,
            }),
        }
    }

    fn ids(tabs: &[Tab]) -> Vec<usize> {
        tabs.iter().map(|t| t.id).collect()
    }

    /// Pinning and unpinning work on tabs that were never built, keeping
    /// pins ahead of the rest.
    #[test]
    fn pin_moves_an_unbuilt_tab_across_the_pinned_group() {
        let mut tabs = vec![unbuilt(1, true), unbuilt(2, false), unbuilt(3, false)];
        toggle_pin(&mut tabs, 3);
        assert_eq!(ids(&tabs), [1, 3, 2]);
        assert!(tabs[1].pinned);
        toggle_pin(&mut tabs, 1);
        assert_eq!(ids(&tabs), [3, 1, 2]);
        assert!(!tabs[1].pinned);
        toggle_pin(&mut tabs, 99);
        assert_eq!(ids(&tabs), [3, 1, 2]);
    }

    /// Moving swaps neighbours of the same group only, and stops at the ends.
    #[test]
    fn move_swaps_within_a_group_and_stops_at_edges() {
        let mut tabs = vec![unbuilt(1, true), unbuilt(2, false), unbuilt(3, false)];
        move_by(&mut tabs, 2, -1);
        assert_eq!(ids(&tabs), [1, 2, 3], "no crossing the pinned boundary");
        move_by(&mut tabs, 2, 1);
        assert_eq!(ids(&tabs), [1, 3, 2]);
        move_by(&mut tabs, 2, 1);
        assert_eq!(ids(&tabs), [1, 3, 2], "no move past the end");
        move_by(&mut tabs, 1, -1);
        assert_eq!(ids(&tabs), [1, 3, 2], "no move before the start");
    }

    /// Reordering a workspace's tabs never changes which tab is the active one.
    #[test]
    fn reordering_keeps_the_active_tab() {
        let mut space = Space::new("w".into(), PathBuf::from("/w"));
        space.tabs = vec![unbuilt(1, false), unbuilt(2, false), unbuilt(3, false)];
        space.active = 2;
        reorder_keeping_active(&mut space, |tabs| move_by(tabs, 3, -1));
        assert_eq!(ids(&space.tabs), [1, 3, 2]);
        assert_eq!(space.tabs[space.active].id, 3);
        reorder_keeping_active(&mut space, |tabs| toggle_pin(tabs, 2));
        assert_eq!(space.tabs[space.active].id, 3);
    }
}
