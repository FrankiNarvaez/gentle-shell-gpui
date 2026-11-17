//! Generative UI: surfaces the agent shows in the conversation with the
//! `gentle_ui` tool (from the Pi extension the app ships, `gentle-ui.ts`),
//! drawn natively and edited by the user; the edits go back to the agent
//! through the extension's `/gentle-ui:event` command, which answers each
//! one ([`GenUi::outcome`]). Kinds: the diagram ([`DiagramView`]), the
//! panels ([`PanelView`]: table, form, checklist, kanban, chart) and `html`
//! ([`HtmlView`]), an agent-written page run in an isolated web view that
//! speaks MCP Apps.
//!
//! Nothing is stored apart from the conversation: every version is a tool
//! result's or a `gentle-ui.event` message's `details`, so [`GenUi::sync`]
//! rebuilds the surfaces from the transcript, live or replayed.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use gentle_core::genui::live::Source;
use gentle_core::genui::{self, Snapshot};
use gentle_core::transcript::{Item, ToolStatus};
use gpui_kit::*;

mod chart;
mod checklist;
mod details;
mod diagram;
mod extension;
mod form;
mod html;
mod kanban;
mod live;
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
pub use html::HtmlView;
use live::SharedEnv;
pub use live::todo_records;
pub use panel::PanelView;

/// The id of the app's own todo checklist (never a `gentle_ui` id: those
/// are slugs without ":").
pub const TODO_SURFACE: &str = "app:todo";

/// One surface's live view, of any kind. Every kind emits [`DiagramEvent`].
#[derive(Clone)]
pub enum Surface {
    Diagram(Entity<DiagramView>),
    /// Table, form, checklist, kanban and chart.
    Panel(Entity<PanelView>),
    Html(Entity<HtmlView>),
}

impl Surface {
    fn new(snapshot: &Snapshot, cx: &mut App) -> Self {
        match snapshot.kind.as_str() {
            "html" => Self::Html(cx.new(|cx| HtmlView::new(snapshot, cx))),
            kind if panel::handles(kind) => Self::Panel(cx.new(|cx| PanelView::new(snapshot, cx))),
            _ => Self::Diagram(cx.new(|cx| DiagramView::new(snapshot, cx))),
        }
    }

    /// Whether this view draws `kind` (a reloaded transcript may reuse an id;
    /// an id shown again as another kind gets a new surface).
    fn fits(&self, kind: &str, cx: &App) -> bool {
        match self {
            Self::Diagram(_) => kind != "html" && !panel::handles(kind),
            Self::Panel(view) => view.read(cx).kind() == kind,
            Self::Html(_) => kind == "html",
        }
    }

    pub fn entity_id(&self) -> EntityId {
        match self {
            Self::Diagram(view) => view.entity_id(),
            Self::Panel(view) => view.entity_id(),
            Self::Html(view) => view.entity_id(),
        }
    }

    pub fn id(&self, cx: &App) -> String {
        match self {
            Self::Diagram(view) => view.read(cx).id().to_string(),
            Self::Panel(view) => view.read(cx).id().to_string(),
            Self::Html(view) => view.read(cx).id().to_string(),
        }
    }

    fn set_snapshot(&self, snapshot: &Snapshot, reset: bool, cx: &mut App) {
        match self {
            Self::Diagram(view) => {
                view.update(cx, |view, cx| view.set_snapshot(snapshot, reset, cx))
            }
            Self::Panel(view) => view.update(cx, |view, cx| view.set_snapshot(snapshot, reset, cx)),
            Self::Html(view) => view.update(cx, |view, cx| view.set_snapshot(snapshot, reset, cx)),
        }
    }

    fn set_expanded(&self, expanded: bool, cx: &mut App) {
        match self {
            Self::Diagram(view) => view.update(cx, |view, cx| view.set_expanded(expanded, cx)),
            Self::Panel(view) => view.update(cx, |view, cx| view.set_expanded(expanded, cx)),
            Self::Html(view) => view.update(cx, |view, cx| view.set_expanded(expanded, cx)),
        }
    }

    /// Whether Gentle Shell ran the surface's last [`DiagramEvent::Send`].
    pub fn sent(&self, result: Result<(), String>, cx: &mut App) {
        match self {
            Self::Diagram(view) => view.update(cx, |view, cx| view.sent(result, cx)),
            Self::Panel(view) => view.update(cx, |view, cx| view.sent(result, cx)),
            Self::Html(view) => view.update(cx, |view, cx| view.sent(result, cx)),
        }
    }

    /// The extension's answer to the surface's last [`DiagramEvent::Send`].
    fn outcome(&self, outcome: &genui::Outcome, cx: &mut App) {
        match self {
            Self::Diagram(view) => view.update(cx, |view, cx| view.outcome(outcome, cx)),
            Self::Panel(view) => view.update(cx, |view, cx| view.outcome(outcome, cx)),
            Self::Html(view) => view.update(cx, |view, cx| view.outcome(outcome, cx)),
        }
    }

    /// Something is drawn over the surface or its conversation is off
    /// screen: only a native web view (html) needs to know.
    pub fn set_obscured(&self, obscured: bool, cx: &mut App) {
        if let Self::Html(view) = self {
            view.update(cx, |view, cx| view.set_obscured(obscured, cx));
        }
    }

    pub fn emit(&self, event: DiagramEvent, cx: &mut App) {
        match self {
            Self::Diagram(view) => view.update(cx, |_, cx| cx.emit(event)),
            Self::Panel(view) => view.update(cx, |_, cx| cx.emit(event)),
            Self::Html(view) => view.update(cx, |_, cx| cx.emit(event)),
        }
    }

    /// The card's height in the conversation (px at 100 %).
    fn card_height(&self, cx: &App) -> f32 {
        match self {
            Self::Diagram(_) => view::CARD_HEIGHT,
            Self::Panel(view) => view.read(cx).card_height(),
            Self::Html(_) => html::CARD_HEIGHT,
        }
    }

    pub fn view(&self) -> AnyView {
        match self {
            Self::Diagram(view) => view.clone().into(),
            Self::Panel(view) => view.clone().into(),
            Self::Html(view) => view.clone().into(),
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
    /// Something is drawn over the conversation, or it isn't on screen.
    obscured: bool,
    /// Created since the conversation last subscribed to them.
    fresh: Vec<Surface>,
    subscriptions: HashMap<EntityId, Subscription>,
    /// The live data, workspace and allowed commands every live panel shares.
    env: SharedEnv,
    /// The agent's todo list as a checklist ("Show as checklist"), the
    /// app's own surface: no tool call, never in the transcript.
    todo: Option<Surface>,
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
        .unwrap_or_else(|| match args["kind"].as_str().unwrap_or_default() {
            "html" => "HTML",
            kind => panel::default_title(kind),
        })
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
                Some(surface) if surface.fits(&snapshot.kind, cx) => {
                    surface.set_snapshot(&snapshot, reset, cx)
                }
                _ => {
                    if let Some(old) = self.surfaces.remove(&id) {
                        self.subscriptions.remove(&old.entity_id());
                    }
                    let surface = Surface::new(&snapshot, cx);
                    if let Surface::Panel(view) = &surface {
                        let env = self.env.clone();
                        view.update(cx, |view, cx| view.attach(env, cx));
                    }
                    self.fresh.push(surface.clone());
                    self.surfaces.insert(id, surface);
                }
            }
        }
        if self
            .expanded
            .as_ref()
            .is_some_and(|id| !self.surfaces.contains_key(id) && id != TODO_SURFACE)
        {
            self.expanded = None;
        }
    }

    /// Where live git sources and commands run (the conversation's folder).
    pub fn set_workspace(&mut self, dir: &Path) {
        let mut env = self.env.borrow_mut();
        if env.workspace.as_deref() != Some(dir) {
            env.workspace = Some(dir.to_path_buf());
        }
    }

    fn panels(&self) -> impl Iterator<Item = &Entity<PanelView>> {
        self.surfaces
            .values()
            .chain(&self.todo)
            .filter_map(|surface| match surface {
                Surface::Panel(view) => Some(view),
                _ => None,
            })
    }

    /// Whether a live panel (or a kanban card's subagent) shows `source`:
    /// the conversation builds its data only then.
    pub fn wants(&self, source: Source, cx: &App) -> bool {
        self.panels().any(|view| view.read(cx).wants(source))
    }

    /// `source`'s latest data; the panels showing it follow when it changed.
    pub fn feed(&mut self, source: Source, data: serde_json::Value, cx: &mut App) {
        {
            let mut env = self.env.borrow_mut();
            if env.feeds.get(&source) == Some(&data) {
                return;
            }
            env.feeds.insert(source, data);
        }
        let views: Vec<Entity<PanelView>> = self.panels().cloned().collect();
        for view in views {
            view.update(cx, |view, cx| {
                if view.wants(source) {
                    view.feed(source, cx);
                }
            });
        }
    }

    /// The Todos island's "Show as checklist": the agent's todo list,
    /// live and read-only, in place of the conversation.
    pub fn show_todo(&mut self, cx: &mut App) {
        if self.todo.is_none() {
            let view = cx.new(|cx| PanelView::todo_list(TODO_SURFACE, cx));
            let env = self.env.clone();
            view.update(cx, |view, cx| view.attach(env, cx));
            let surface = Surface::Panel(view);
            self.fresh.push(surface.clone());
            self.todo = Some(surface);
        }
        self.set_expanded(Some(TODO_SURFACE.to_string()), cx);
    }

    /// The extension's answer to a surface's sent edits (its
    /// [`genui::OUTCOME_WIDGET`] line).
    pub fn outcome(&self, line: &str, cx: &mut App) {
        let Some(outcome) = genui::outcome(line) else {
            return;
        };
        if let Some(surface) = self.surfaces.get(&outcome.id) {
            surface.outcome(&outcome, cx);
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
        for (key, surface) in &self.surfaces {
            let on = id.as_ref() == Some(key);
            surface.set_obscured(self.obscured, cx);
            surface.set_expanded(on, cx);
        }
        if let Some(todo) = &self.todo {
            todo.set_obscured(self.obscured, cx);
            todo.set_expanded(id.as_deref() == Some(TODO_SURFACE), cx);
        }
        self.expanded = id.filter(|id| {
            self.surfaces.contains_key(id) || (id == TODO_SURFACE && self.todo.is_some())
        });
    }

    pub fn expanded(&self) -> Option<Surface> {
        let id = self.expanded.as_ref()?;
        match self.surfaces.get(id) {
            Some(surface) => Some(surface.clone()),
            None if id == TODO_SURFACE => self.todo.clone(),
            None => None,
        }
    }

    /// Something is drawn over the conversation (a menu over the expanded
    /// surface) or it isn't on screen: native web views hide.
    pub fn set_obscured(&mut self, obscured: bool, cx: &mut App) {
        if self.obscured != obscured {
            self.obscured = obscured;
            if let Some(surface) = self.expanded() {
                surface.set_obscured(obscured, cx);
            }
        }
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
            Some((id, surface)) if self.expanded.as_ref() == Some(id) => {
                view::expanded_row(surface.clone(), &title)
            }
            Some((_, surface)) => div()
                .w_full()
                .h(rems(surface.card_height(cx) / 16.))
                .child(
                    surface
                        .view()
                        .cached(StyleRefinement::default().size_full()),
                )
                .into_any_element(),
            None => view::superseded_row(&title, snapshot.map_or(0, |s| s.version), false),
        }
    }

    /// The card for the user's sent edits (a `gentle-ui.event` message).
    pub fn render_event(details: &serde_json::Value) -> AnyElement {
        view::event_card(details)
    }
}
