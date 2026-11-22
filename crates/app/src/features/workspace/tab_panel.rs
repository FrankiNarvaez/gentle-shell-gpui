//! Tab placement and the tab operations the vertical sidebar shares with the
//! horizontal strip. It shares the window's existing tabs, panes and
//! persisted workspaces; no second tab model is created. The sidebar itself
//! is drawn in `sidebars/`.

use super::*;

fn platform_select_modifier(event: &ClickEvent, cx: &App) -> bool {
    event.modifiers().secondary(cx)
}

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

    // ── Tab multi-selection ───────────────────────────────────────────────

    pub(super) fn selected_tab_count(&self, space: usize) -> usize {
        self.tab_selection
            .as_ref()
            .filter(|s| s.space == space)
            .map(|s| s.selected.len())
            .unwrap_or(0)
    }

    pub(super) fn is_tab_selected(&self, space: usize, id: usize) -> bool {
        self.tab_selection
            .as_ref()
            .filter(|s| s.space == space)
            .is_some_and(|s| s.selected.contains(&id))
    }

    pub(super) fn selected_tab_ids(&self, space: usize) -> Vec<usize> {
        self.tab_selection
            .as_ref()
            .filter(|s| s.space == space)
            .map(|s| s.selected.iter().copied().collect())
            .unwrap_or_default()
    }

    pub(super) fn clear_tab_selection(&mut self, cx: &mut Context<Self>) {
        if self.tab_selection.take().is_some() {
            cx.notify();
        }
    }

    /// Remove tab ids that no longer exist in their workspace. Call after
    /// any operation that may have removed tabs.
    pub(super) fn prune_tab_selection(&mut self, cx: &mut Context<Self>) {
        let Some(sel) = self.tab_selection.as_ref() else {
            return;
        };
        let space = sel.space;
        let Some(existing): Option<std::collections::HashSet<usize>> = self
            .spaces
            .get(space)
            .map(|s| s.tabs.iter().map(|t| t.id).collect())
        else {
            self.tab_selection = None;
            cx.notify();
            return;
        };
        let before = sel.selected.len();
        let selected: std::collections::HashSet<usize> =
            sel.selected.intersection(&existing).copied().collect();
        if selected.is_empty() {
            self.tab_selection = None;
        } else if selected.len() != before {
            self.tab_selection = Some(TabSelection {
                space,
                anchor: sel.anchor,
                selected,
            });
        } else {
            return;
        }
        cx.notify();
    }

    /// Handle a left click on a tab of the shown workspace. Double-click is
    /// handled separately and always starts a rename.
    pub(super) fn on_tab_click(
        &mut self,
        index: usize,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.space().tabs.get(index).is_none() {
            return;
        };
        let platform = platform_select_modifier(event, cx);
        let shift = event.modifiers().shift;
        if platform || shift {
            self.update_tab_selection(index, platform, shift, cx);
        } else {
            self.clear_tab_selection(cx);
            self.activate(index, window, cx);
        }
    }

    fn update_tab_selection(
        &mut self,
        index: usize,
        platform: bool,
        shift: bool,
        cx: &mut Context<Self>,
    ) {
        let space = self.current;
        let Some(tab_id) = self.space().tabs.get(index).map(|t| t.id) else {
            return;
        };
        let tabs: Vec<usize> = self.space().tabs.iter().map(|t| t.id).collect();
        let (selected, anchor) = self
            .tab_selection
            .as_ref()
            .filter(|s| s.space == space)
            .map(|s| {
                (
                    s.selected.iter().copied().collect::<Vec<_>>(),
                    Some(s.anchor),
                )
            })
            .unwrap_or_default();
        let update =
            gentle_core::workspaces::select_tab(&tabs, &selected, anchor, tab_id, platform, shift);
        if update.selected.is_empty() {
            self.tab_selection = None;
        } else {
            self.tab_selection = Some(TabSelection {
                space,
                anchor: update.anchor.unwrap_or(tab_id),
                selected: update.selected.iter().copied().collect(),
            });
        }
        cx.notify();
    }

    /// Pin or unpin every selected tab of `space`. The decision is
    /// deterministic: if any selected tab is unpinned, pin all of them;
    /// otherwise unpin all of them.
    pub(super) fn toggle_selected_tab_pins(
        &mut self,
        space: usize,
        ids: Vec<usize>,
        cx: &mut Context<Self>,
    ) {
        let Some(new_pinned) = gentle_core::workspaces::pin_state_for_selection(
            |id| {
                self.spaces
                    .get(space)
                    .and_then(|s| s.tabs.iter().find(|t| t.id == id))
                    .is_some_and(|t| t.pinned)
            },
            &ids,
        ) else {
            return;
        };
        self.with_reordered_tabs(
            space,
            |tabs| {
                for tab in tabs.iter_mut() {
                    if ids.contains(&tab.id) {
                        tab.pinned = new_pinned;
                    }
                }
                // Keep pinned tabs ahead while preserving relative order inside
                // each group. reorder_keeping_active preserves the shown tab.
                let mut pinned: Vec<Tab> = tabs.iter().filter(|t| t.pinned).cloned().collect();
                let mut unpinned: Vec<Tab> = tabs.iter().filter(|t| !t.pinned).cloned().collect();
                tabs.clear();
                tabs.append(&mut pinned);
                tabs.append(&mut unpinned);
            },
            cx,
        );
    }

    /// Close every selected tab of `space`. Built tabs follow the existing
    /// confirmation rules; unbuilt tabs close without confirmation and are
    /// never built.
    pub(super) fn close_selected_tabs(
        &mut self,
        space: usize,
        ids: Vec<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(space_ref) = self.spaces.get(space) else {
            return;
        };
        let mut built_leaves: Vec<PaneId> = Vec::new();
        let mut to_close: Vec<usize> = Vec::new();
        for tab in &space_ref.tabs {
            if ids.contains(&tab.id) {
                to_close.push(tab.id);
                if tab.is_built() {
                    built_leaves.extend(tab.leaves());
                }
            }
        }
        if to_close.is_empty() {
            return;
        }

        let unsaved = self.unsaved_titles(built_leaves.clone(), cx);
        let busy = self.busy_titles(built_leaves);

        if unsaved.is_empty() && busy.is_empty() {
            self.close_tab_ids_now(space, &to_close, window, cx);
        } else {
            self.confirm_close(unsaved, busy, window, cx, move |this, window, cx| {
                // Recompute which selected tabs still exist; the dialog may
                // have stayed open across other changes.
                let mut remaining: Vec<usize> = Vec::new();
                if let Some(s) = this.spaces.get(space) {
                    for tab in &s.tabs {
                        if ids.contains(&tab.id) {
                            remaining.push(tab.id);
                        }
                    }
                }
                this.close_tab_ids_now(space, &remaining, window, cx);
            });
        }
    }

    /// Close a set of tabs by id, built or unbuilt, in one batch.
    fn close_tab_ids_now(
        &mut self,
        space: usize,
        ids: &[usize],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(space_ref) = self.spaces.get(space) else {
            return;
        };
        let mut removals: Vec<(usize, bool)> = Vec::new();
        for (i, tab) in space_ref.tabs.iter().enumerate() {
            if ids.contains(&tab.id) {
                removals.push((i, tab.is_built()));
            }
        }
        if removals.is_empty() {
            return;
        }

        let original_len = space_ref.tabs.len();
        let original_active = space_ref.active;

        // Remember every close for reopen.
        for (idx, is_built) in &removals {
            let tab = &self.spaces[space].tabs[*idx];
            if *is_built {
                let leaves = tab.leaves();
                self.remember_close(space, *idx, &leaves, cx);
            } else if let Some(saved) = tab.state.unbuilt().cloned() {
                let title = saved
                    .layout
                    .leaves()
                    .get(
                        saved
                            .focused
                            .min(saved.layout.leaves().len().saturating_sub(1)),
                    )
                    .map(super::sidebars::saved_title)
                    .unwrap_or_default();
                if let Some(closed) =
                    closed::Closed::unbuilt(space, tab.id, *idx, tab.pinned, saved, title)
                {
                    if self.closed.len() == closed::KEEP {
                        self.closed.remove(0);
                    }
                    self.closed.push(closed);
                }
            }
        }

        // Remove from highest index to lowest so indices stay valid.
        let mut indices: Vec<usize> = removals.iter().map(|(i, _)| *i).collect();
        indices.sort_unstable_by(|a, b| b.cmp(a));
        for idx in indices {
            let tab = self.spaces[space].tabs.remove(idx);
            if tab.is_built() {
                for pane in tab.leaves() {
                    self.panes.remove(&pane);
                }
            }
        }

        self.finish_multi_tab_close(space, original_len, original_active, &removals, window, cx);
    }

    fn finish_multi_tab_close(
        &mut self,
        space: usize,
        original_len: usize,
        original_active: usize,
        removals: &[(usize, bool)],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let removed: Vec<usize> = removals.iter().map(|(i, _)| *i).collect();
        self.spaces[space].active =
            gentle_core::workspaces::active_after_removals(original_len, original_active, &removed);
        self.prune_tab_selection(cx);
        self.announce_close(window, cx);
        if space == self.current {
            if self.space().tabs.is_empty() {
                let root = self.space().root.clone();
                self.open_tab(PaneKind::Agent, root, None, None, window, cx);
                self.save_now(cx);
            } else {
                self.activate(self.space().active, window, cx);
            }
        } else {
            self.schedule_save(cx);
            cx.notify();
        }
    }

    /// Build the multi-tab context menu for a selection of 2+ tabs.
    pub(super) fn multi_tab_actions_menu(
        menu: PopupMenu,
        weak: WeakEntity<Self>,
        selection: MultiTabMenu,
    ) -> PopupMenu {
        let MultiTabMenu {
            space,
            ids,
            any_unpinned,
            count,
        } = selection;
        let pin_ids = ids.clone();
        let close_ids = ids;
        let pin = weak.clone();
        let close = weak;
        let pin_label = if any_unpinned {
            format!("Pin {count} tabs")
        } else {
            format!("Unpin {count} tabs")
        };
        menu.item(
            PopupMenuItem::new(pin_label.clone())
                .icon(Icon::new(if any_unpinned {
                    IconName::Pin
                } else {
                    IconName::PinOff
                }))
                .on_click(move |_, _, cx| {
                    let _ = pin.update(cx, |this, cx| {
                        this.toggle_selected_tab_pins(space, pin_ids.clone(), cx)
                    });
                }),
        )
        .separator()
        .item(
            PopupMenuItem::new(format!("Close {count} tabs"))
                .icon(Icon::new(IconName::X))
                .on_click(move |_, window, cx| {
                    let _ = close.update(cx, |this, cx| {
                        this.close_selected_tabs(space, close_ids.clone(), window, cx)
                    });
                }),
        )
    }
}

/// A multi-selection of tabs shown in the context menu.
#[derive(Clone)]
pub(super) struct MultiTabMenu {
    pub space: usize,
    pub ids: Vec<usize>,
    pub any_unpinned: bool,
    pub count: usize,
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

/// Keep the selected tab after removal, or select its neighbour if it closed.
pub(super) fn after_removal(space: &mut Space, index: usize) {
    let active = space.active - usize::from(index < space.active);
    space.active = active.min(space.tabs.len().saturating_sub(1));
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Space, Tab, TabState, after_removal, move_by, reorder_keeping_active, toggle_pin};
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

    #[test]
    fn closing_above_the_active_tab_keeps_it_selected() {
        let mut space = Space::new("w".into(), PathBuf::from("/w"));
        space.tabs = vec![unbuilt(1, false), unbuilt(2, false), unbuilt(3, false)];
        space.active = 1;
        space.tabs.remove(0);
        after_removal(&mut space, 0);
        assert_eq!(ids(&space.tabs), [2, 3]);
        assert_eq!(space.tabs[space.active].id, 2);
    }

    #[test]
    fn closing_the_last_active_tab_clamps_active() {
        let mut space = Space::new("w".into(), PathBuf::from("/w"));
        space.tabs = vec![unbuilt(1, false), unbuilt(2, false)];
        space.active = 1;
        space.tabs.remove(1);
        after_removal(&mut space, 1);
        assert_eq!(ids(&space.tabs), [1]);
        assert_eq!(space.active, 0);
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

    /// Pinning a multi-selection sets every selected tab to the same pin
    /// state, keeps pinned tabs ahead, and leaves the active tab selected.
    #[test]
    fn multi_pin_pins_all_when_any_unpinned_and_keeps_active() {
        use gentle_core::workspaces::pin_state_for_selection;

        let mut space = Space::new("w".into(), PathBuf::from("/w"));
        space.tabs = vec![
            unbuilt(1, false),
            unbuilt(2, true),
            unbuilt(3, false),
            unbuilt(4, true),
        ];
        space.active = 2;

        let selected = vec![2, 3];
        let new_pinned = pin_state_for_selection(
            |id| {
                space
                    .tabs
                    .iter()
                    .find(|t| t.id == id)
                    .is_some_and(|t| t.pinned)
            },
            &selected,
        )
        .unwrap();
        assert!(new_pinned, "one unpinned tab => pin the whole selection");

        reorder_keeping_active(&mut space, |tabs| {
            for tab in tabs.iter_mut() {
                if selected.contains(&tab.id) {
                    tab.pinned = new_pinned;
                }
            }
            let mut pinned: Vec<Tab> = tabs.iter().filter(|t| t.pinned).cloned().collect();
            let mut unpinned: Vec<Tab> = tabs.iter().filter(|t| !t.pinned).cloned().collect();
            tabs.clear();
            tabs.append(&mut pinned);
            tabs.append(&mut unpinned);
        });

        assert_eq!(ids(&space.tabs), [2, 3, 4, 1]);
        assert!(space.tabs.iter().take(3).all(|t| t.pinned));
        assert!(!space.tabs[3].pinned);
        assert_eq!(space.tabs[space.active].id, 3, "active tab stays selected");
    }

    /// Removing several tabs at once keeps the active index valid.
    #[test]
    fn multi_close_keeps_active_valid() {
        use gentle_core::workspaces::active_after_removals;

        let mut space = Space::new("w".into(), PathBuf::from("/w"));
        space.tabs = vec![
            unbuilt(1, false),
            unbuilt(2, false),
            unbuilt(3, false),
            unbuilt(4, false),
        ];
        space.active = 2;
        let removed = vec![0, 2];
        for idx in gentle_core::workspaces::removal_order(&removed) {
            space.tabs.remove(idx);
        }
        space.active = active_after_removals(4, 2, &removed);
        assert_eq!(ids(&space.tabs), [2, 4]);
        assert_eq!(space.active, 1);
    }
}
