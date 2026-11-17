//! Generative UI: surfaces the agent shows in the conversation with the
//! `gentle_ui` tool (from the Pi extension the app ships, `gentle-ui.ts`),
//! drawn natively and edited by the user; the edits go back to the agent
//! through the extension's `/gentle-ui:event` command. Kinds: the diagram
//! ([`DiagramView`]) and the panels ([`PanelView`]: table, form, checklist,
//! kanban, chart).
//!
//! Nothing is stored apart from the conversation: every version is a tool
//! result's or a `gentle-ui.event` message's `details`, so [`GenUi::sync`]
//! rebuilds the surfaces from the transcript, live or replayed.

use std::collections::{HashMap, HashSet};

use gentle_core::genui::{self, Snapshot};
use gentle_core::transcript::{Item, ToolStatus};
use gpui_kit::*;

mod chart;
mod checklist;
mod details;
mod diagram;
mod extension;
mod form;
mod kanban;
mod motion;
mod paint;
mod panel;
mod pointer;
mod table;
#[cfg(test)]
mod tests;
mod view;

pub use diagram::{DiagramEvent, DiagramView};
pub use extension::install;
pub use panel::PanelView;

/// One surface's live view. Every kind emits [`DiagramEvent`].
#[derive(Clone)]
pub enum Surface {
    Diagram(Entity<DiagramView>),
    /// Table, form, checklist, kanban and chart.
    Panel(Entity<PanelView>),
}

impl Surface {
    fn new(snapshot: &Snapshot, cx: &mut App) -> Self {
        if panel::handles(&snapshot.kind) {
            Surface::Panel(cx.new(|cx| PanelView::new(snapshot, cx)))
        } else {
            Surface::Diagram(cx.new(|cx| DiagramView::new(snapshot, cx)))
        }
    }

    /// Whether this view draws `kind` (a reloaded transcript may reuse an id).
    fn fits(&self, kind: &str, cx: &App) -> bool {
        match self {
            Surface::Diagram(_) => !panel::handles(kind),
            Surface::Panel(view) => view.read(cx).kind() == kind,
        }
    }

    pub fn entity_id(&self) -> EntityId {
        match self {
            Surface::Diagram(view) => view.entity_id(),
            Surface::Panel(view) => view.entity_id(),
        }
    }

    pub fn id(&self, cx: &App) -> String {
        match self {
            Surface::Diagram(view) => view.read(cx).id().to_string(),
            Surface::Panel(view) => view.read(cx).id().to_string(),
        }
    }

    fn set_snapshot(&self, snapshot: &Snapshot, reset: bool, cx: &mut App) {
        match self {
            Surface::Diagram(view) => {
                view.update(cx, |view, cx| view.set_snapshot(snapshot, reset, cx))
            }
            Surface::Panel(view) => {
                view.update(cx, |view, cx| view.set_snapshot(snapshot, reset, cx))
            }
        }
    }

    fn set_expanded(&self, expanded: bool, cx: &mut App) {
        match self {
            Surface::Diagram(view) => view.update(cx, |view, cx| view.set_expanded(expanded, cx)),
            Surface::Panel(view) => view.update(cx, |view, cx| view.set_expanded(expanded, cx)),
        }
    }

    /// The extension's answer to [`DiagramEvent::Send`].
    pub fn sent(&self, result: Result<(), String>, cx: &mut App) {
        match self {
            Surface::Diagram(view) => view.update(cx, |view, cx| view.sent(result, cx)),
            Surface::Panel(view) => view.update(cx, |view, cx| view.sent(result, cx)),
        }
    }

    /// Ask the conversation to show it back in its card.
    pub fn collapse(&self, cx: &mut App) {
        match self {
            Surface::Diagram(view) => view.update(cx, |_, cx| cx.emit(DiagramEvent::Collapse)),
            Surface::Panel(view) => view.update(cx, |_, cx| cx.emit(DiagramEvent::Collapse)),
        }
    }

    /// The card's height in the conversation (px at 100 %).
    fn height(&self, cx: &App) -> f32 {
        match self {
            Surface::Diagram(_) => view::CARD_HEIGHT,
            Surface::Panel(view) => view.read(cx).card_height(),
        }
    }

    pub fn view(&self) -> AnyView {
        match self {
            Surface::Diagram(view) => view.clone().into(),
            Surface::Panel(view) => view.clone().into(),
        }
    }
}

/// The surfaces of one conversation.
#[derive(Default)]
pub struct GenUi {
    surfaces: HashMap<String, Surface>,
    /// The item each surface's live card is drawn at: its latest
    /// `gentle_ui` tool item. Earlier items of the same id are superseded.
    homes: HashMap<usize, String>,
    /// Surfaces the agent closed, by the item that closed them.
    closed: HashSet<usize>,
    expanded: Option<String>,
    /// Created since the conversation last subscribed to them.
    fresh: Vec<Surface>,
    subscriptions: HashMap<EntityId, Subscription>,
}

/// Whether `item` carries a surface version (or will).
fn is_surface(item: &Item) -> bool {
    match item {
        Item::Tool { name, .. } => name == genui::TOOL,
        Item::Custom { kind, .. } => kind == genui::EVENT_MESSAGE,
        _ => false,
    }
}

fn tool_title(args: &serde_json::Value, snapshot: Option<&Snapshot>) -> String {
    snapshot
        .map(|s| s.title.as_str())
        .or(args["title"].as_str())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| panel::default_title(args["kind"].as_str().unwrap_or_default()))
        .to_string()
}

impl GenUi {
    /// Follow the transcript after items `from..` changed (`reset`: it was
    /// replaced). Cheap when none of them is a surface.
    pub fn sync(&mut self, items: &[Item], from: usize, reset: bool, cx: &mut App) {
        if !reset && !items.get(from..).unwrap_or_default().iter().any(is_surface) {
            return;
        }
        // Each id's latest version and live card, in conversation order.
        let mut latest: HashMap<String, Snapshot> = HashMap::new();
        let mut homes: HashMap<String, usize> = HashMap::new();
        self.closed.clear();
        for (ix, item) in items.iter().enumerate() {
            let (snapshot, args) = match item {
                Item::Tool {
                    name,
                    details,
                    args,
                    ..
                } if name == genui::TOOL => (genui::snapshot(details), Some(args)),
                Item::Custom { kind, details, .. } if kind == genui::EVENT_MESSAGE => {
                    (genui::snapshot(details), None)
                }
                _ => continue,
            };
            if let Some(args) = args {
                let id = snapshot.as_ref().map(|s| s.id.clone()).or_else(|| {
                    args["id"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .map(str::to_string)
                });
                if let Some(id) = id {
                    homes.insert(id, ix);
                }
            }
            if let Some(snapshot) = snapshot {
                if snapshot.closed {
                    self.closed.insert(ix);
                }
                latest.insert(snapshot.id.clone(), snapshot);
            }
        }
        self.homes = homes.into_iter().map(|(id, ix)| (ix, id)).collect();
        let gone: Vec<String> = self
            .surfaces
            .keys()
            .filter(|id| latest.get(*id).is_none_or(|s| s.closed))
            .cloned()
            .collect();
        for id in gone {
            if let Some(view) = self.surfaces.remove(&id) {
                self.subscriptions.remove(&view.entity_id());
            }
        }
        for (id, snapshot) in latest {
            if snapshot.closed {
                continue;
            }
            match self.surfaces.get(&id) {
                Some(view) if view.fits(&snapshot.kind, cx) => {
                    view.set_snapshot(&snapshot, reset, cx)
                }
                _ => {
                    if let Some(old) = self.surfaces.remove(&id) {
                        self.subscriptions.remove(&old.entity_id());
                    }
                    let view = Surface::new(&snapshot, cx);
                    self.fresh.push(view.clone());
                    self.surfaces.insert(id, view);
                }
            }
        }
        if self
            .expanded
            .as_ref()
            .is_some_and(|id| !self.surfaces.contains_key(id))
        {
            self.expanded = None;
        }
    }

    /// The extension's answer to a surface's sent edits (its
    /// [`genui::OUTCOME_WIDGET`] line).
    pub fn outcome(&self, line: &str, cx: &mut App) {
        let Some(outcome) = genui::outcome(line) else {
            return;
        };
        if let Some(view) = self.surfaces.get(&outcome.id) {
            view.update(cx, |view, cx| view.outcome(&outcome, cx));
        }
    }

    /// Surfaces created since the last call, for the conversation to
    /// subscribe to (and hand back with [`Self::keep`]).
    pub fn take_fresh(&mut self) -> Vec<Surface> {
        std::mem::take(&mut self.fresh)
    }

    pub fn keep(&mut self, view: EntityId, subscription: Subscription) {
        self.subscriptions.insert(view, subscription);
    }

    /// Show `id` in place of the conversation (`None`: back to it).
    pub fn set_expanded(&mut self, id: Option<String>, cx: &mut App) {
        for (surface, view) in &self.surfaces {
            let on = id.as_ref() == Some(surface);
            view.set_expanded(on, cx);
        }
        self.expanded = id.filter(|id| self.surfaces.contains_key(id));
    }

    pub fn expanded(&self) -> Option<Surface> {
        self.surfaces.get(self.expanded.as_ref()?).cloned()
    }

    /// What a `gentle_ui` tool item shows: the live surface, or a row for
    /// a version still arriving, failed, closed or superseded.
    pub fn render_tool(&self, ix: usize, item: &Item, cx: &App) -> AnyElement {
        let Item::Tool {
            args,
            status,
            output,
            details,
            ..
        } = item
        else {
            return div().into_any_element();
        };
        let snapshot = genui::snapshot(details);
        let title = tool_title(args, snapshot.as_ref());
        match status {
            ToolStatus::Running => return view::drawing_card(&title),
            ToolStatus::Failed => return view::failed_card(&title, output),
            ToolStatus::Done => {}
        }
        if self.closed.contains(&ix) {
            return view::superseded_row(&title, 0, true);
        }
        let live = self
            .homes
            .get(&ix)
            .and_then(|id| Some((id, self.surfaces.get(id)?)));
        match live {
            Some((id, view)) if self.expanded.as_ref() == Some(id) => {
                view::expanded_row(view.clone(), &title)
            }
            Some((_, view)) => div()
                .w_full()
                .h(rems(view.height(cx) / 16.))
                .child(view.view().cached(StyleRefinement::default().size_full()))
                .into_any_element(),
            None => view::superseded_row(&title, snapshot.map_or(0, |s| s.version), false),
        }
    }

    /// The card for the user's sent edits (a `gentle-ui.event` message).
    pub fn render_event(details: &serde_json::Value) -> AnyElement {
        view::event_card(details)
    }
}
