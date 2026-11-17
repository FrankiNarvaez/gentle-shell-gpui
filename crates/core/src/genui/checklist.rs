//! The checklist kind: items with one level of sub-items that the user
//! checks off, adds, edits, removes and reorders (plans, TODOs). With
//! `run` on it is a plan the user drives: items carry a [`Status`], the
//! user skips and reorders steps and asks the agent to run them
//! ([`Change::Run`], [`Change::Stop`]), and the agent reports each step's
//! status back with `update`.

use serde_json::{Map, Value, json};

use super::diagram::{Errors, MAX_TEXT, array, id_of, text, truncate};
use super::edits::{Editable, fresh_id, list_names, plural};
use super::live::Live;
use super::refs::CodeRef;

pub const MAX_ITEMS: usize = 200;
pub const MAX_ITEM_LABEL: usize = 200;

/// Where an item is in a run plan. `done` is `Done`; the others are only
/// written out when the item has them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Pending,
    Running,
    Done,
    Failed,
    Skipped,
}

impl Status {
    pub const ALL: [Status; 5] = [
        Status::Pending,
        Status::Running,
        Status::Done,
        Status::Failed,
        Status::Skipped,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Running => "running",
            Status::Done => "done",
            Status::Failed => "failed",
            Status::Skipped => "skipped",
        }
    }

    pub fn parse(name: &str) -> Option<Status> {
        Status::ALL
            .into_iter()
            .find(|s| s.name().eq_ignore_ascii_case(name.trim()))
    }

    /// Implied by `done` alone (so not written out).
    fn plain(self) -> bool {
        matches!(self, Status::Pending | Status::Done)
    }
}

/// A run plan's state (`spec.run`): `true` or `"idle"` turns it on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Run {
    /// On, nothing asked for.
    Idle,
    /// The user asked the agent to work through every pending step.
    All,
    /// The user asked for one step.
    Step,
}

impl Run {
    pub fn name(self) -> &'static str {
        match self {
            Run::Idle => "idle",
            Run::All => "all",
            Run::Step => "step",
        }
    }

    /// `None`: off (absent, false, null, or a word it doesn't know).
    fn parse(value: Option<&Value>) -> Option<Run> {
        match value? {
            Value::Bool(true) => Some(Run::Idle),
            Value::String(s) => [Run::Idle, Run::All, Run::Step]
                .into_iter()
                .find(|r| r.name().eq_ignore_ascii_case(s.trim())),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: String,
    pub label: String,
    /// `status == Done`, kept in step with it.
    pub done: bool,
    pub status: Status,
    pub note: Option<String>,
    /// The code this item is about (`ref`).
    pub code_ref: Option<CodeRef>,
    /// Sub-items (only on top-level items: one level).
    pub items: Vec<Item>,
}

impl Item {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            done: false,
            status: Status::Pending,
            note: None,
            code_ref: None,
            items: Vec::new(),
        }
    }

    fn set_status(&mut self, status: Status) {
        self.status = status;
        self.done = status == Status::Done;
    }

    fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("id".into(), json!(self.id));
        map.insert("label".into(), json!(self.label));
        map.insert("done".into(), json!(self.done));
        if !self.status.plain() {
            map.insert("status".into(), json!(self.status.name()));
        }
        if let Some(note) = &self.note {
            map.insert("note".into(), json!(note));
        }
        if let Some(code_ref) = &self.code_ref {
            map.insert("ref".into(), code_ref.to_value());
        }
        if !self.items.is_empty() {
            map.insert(
                "items".into(),
                self.items.iter().map(Item::to_value).collect(),
            );
        }
        Value::Object(map)
    }

    /// This item without its sub-items (what an `add` carries).
    fn alone(&self) -> Item {
        Item {
            items: Vec::new(),
            ..self.clone()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Checklist {
    pub items: Vec<Item>,
    /// `Some`: a run plan the user drives.
    pub run: Option<Run>,
    /// Boxed: rare, and kept small in the edit history's copies.
    pub live: Option<Box<Live>>,
}

/// Where an item sits: its parent (`None`: top level) and index there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place<'a> {
    pub parent: Option<&'a str>,
    pub index: usize,
}

impl Checklist {
    pub fn parse(spec: &Value) -> Result<Checklist, Vec<String>> {
        let Some(spec) = spec.as_object() else {
            return Err(vec!["the spec must be a JSON object".into()]);
        };
        let Some(raw) = spec.get("items").and_then(Value::as_array) else {
            return Err(vec!["\"items\" must be an array".into()]);
        };
        let mut errors = Errors::default();
        let total = raw.len()
            + raw
                .iter()
                .map(|i| array(i.get("items")).len())
                .sum::<usize>();
        if total > MAX_ITEMS {
            errors.push(format!("too many items ({total}, at most {MAX_ITEMS})"));
        }
        let mut list = Checklist::default();
        let mut seen: Vec<String> = Vec::new();
        let mut counter = 0;
        let mut read = |raw: &Value, errors: &mut Errors, seen: &mut Vec<String>| -> Option<Item> {
            counter += 1;
            // "items": ["Write tests", …] is a shorthand for labels.
            let label = match raw {
                Value::String(s) => Some(truncate(s.trim(), MAX_ITEM_LABEL)),
                _ => text(raw.get("label").or_else(|| raw.get("text")), MAX_ITEM_LABEL),
            };
            let id = raw.get("id").and_then(id_of);
            let Some(label) = label.or_else(|| id.clone()) else {
                errors.push(format!("item #{counter} has no label"));
                return None;
            };
            let id =
                id.unwrap_or_else(|| fresh_id(&label, "item", |id| seen.iter().any(|s| s == id)));
            if seen.contains(&id) {
                errors.push(format!("duplicate item id \"{id}\""));
                return None;
            }
            seen.push(id.clone());
            let code_ref = CodeRef::field(raw).unwrap_or_else(|error| {
                errors.push(format!("item \"{id}\": ref {error}"));
                None
            });
            // `status` wins over `done`; an unknown word is ignored.
            let status = raw
                .get("status")
                .and_then(Value::as_str)
                .and_then(Status::parse)
                .unwrap_or(if raw.get("done").and_then(Value::as_bool) == Some(true) {
                    Status::Done
                } else {
                    Status::Pending
                });
            Some(Item {
                id,
                label,
                done: status == Status::Done,
                status,
                note: text(raw.get("note"), MAX_TEXT),
                code_ref,
                items: Vec::new(),
            })
        };
        for top in raw.iter().take(MAX_ITEMS) {
            let Some(mut item) = read(top, &mut errors, &mut seen) else {
                continue;
            };
            for sub in array(top.get("items")) {
                if !array(sub.get("items")).is_empty() {
                    errors.push(format!(
                        "sub-items of \"{}\" can't have their own items (one level only)",
                        item.id
                    ));
                }
                if let Some(sub) = read(sub, &mut errors, &mut seen) {
                    item.items.push(sub);
                }
            }
            list.items.push(item);
        }
        list.run = Run::parse(spec.get("run"));
        match spec.get("live") {
            None | Some(Value::Null) => {}
            Some(raw) => match Live::parse(raw) {
                Ok(live) => list.live = Some(Box::new(live)),
                Err(live_errors) => live_errors.into_iter().for_each(|e| errors.push(e)),
            },
        }
        if !errors.0.is_empty() {
            return Err(errors.0);
        }
        list.settle();
        Ok(list)
    }

    /// A run that asked for something is idle again once nothing runs and
    /// it is through: one step (`Step`), or every pending step (`All`).
    pub fn settle(&mut self) {
        let through = match self.run {
            Some(Run::Step) => true,
            Some(Run::All) => self.next_step().is_none(),
            _ => false,
        };
        if through && self.running().is_none() {
            self.run = Some(Run::Idle);
        }
    }

    /// The steps of a run plan, in order: every item without sub-items.
    pub fn steps(&self) -> Vec<&Item> {
        self.flat()
            .into_iter()
            .map(|(_, item)| item)
            .filter(|item| item.items.is_empty())
            .collect()
    }

    /// `id`'s step number (1-based) in [`Self::steps`].
    pub fn step_number(&self, id: &str) -> Option<usize> {
        self.steps().iter().position(|i| i.id == id).map(|i| i + 1)
    }

    /// The first pending step (skipped, failed and done ones are passed over).
    pub fn next_step(&self) -> Option<&Item> {
        self.steps()
            .into_iter()
            .find(|i| i.status == Status::Pending)
    }

    pub fn running(&self) -> Option<&Item> {
        self.flat()
            .into_iter()
            .map(|(_, item)| item)
            .find(|item| item.status == Status::Running)
    }

    pub fn count(&self) -> usize {
        self.items.iter().map(|i| 1 + i.items.len()).sum()
    }

    pub fn done(&self) -> usize {
        self.items
            .iter()
            .map(|i| usize::from(i.done) + i.items.iter().filter(|s| s.done).count())
            .sum()
    }

    pub fn item(&self, id: &str) -> Option<&Item> {
        self.items
            .iter()
            .flat_map(|i| std::iter::once(i).chain(&i.items))
            .find(|i| i.id == id)
    }

    fn item_mut(&mut self, id: &str) -> Option<&mut Item> {
        for top in &mut self.items {
            if top.id == id {
                return Some(top);
            }
            if let Some(sub) = top.items.iter_mut().find(|s| s.id == id) {
                return Some(sub);
            }
        }
        None
    }

    pub fn place(&self, id: &str) -> Option<Place<'_>> {
        for (i, top) in self.items.iter().enumerate() {
            if top.id == id {
                return Some(Place {
                    parent: None,
                    index: i,
                });
            }
            if let Some(j) = top.items.iter().position(|s| s.id == id) {
                return Some(Place {
                    parent: Some(&top.id),
                    index: j,
                });
            }
        }
        None
    }

    /// Ids of `parent`'s list (`None`: the top level).
    pub fn ids(&self, parent: Option<&str>) -> Vec<String> {
        let list = match parent {
            None => &self.items,
            Some(p) => match self.items.iter().find(|i| i.id == p) {
                Some(p) => &p.items,
                None => return Vec::new(),
            },
        };
        list.iter().map(|i| i.id.clone()).collect()
    }

    /// Every item in reading order with its parent (`None` for top level).
    pub fn flat(&self) -> Vec<(Option<&Item>, &Item)> {
        self.items
            .iter()
            .flat_map(|top| {
                std::iter::once((None, top)).chain(top.items.iter().map(move |s| (Some(top), s)))
            })
            .collect()
    }

    pub fn fresh_id(&self, label: &str) -> String {
        fresh_id(label, "item", |id| self.item(id).is_some())
    }

    fn take(&mut self, id: &str) -> Option<Item> {
        if let Some(i) = self.items.iter().position(|i| i.id == id) {
            return Some(self.items.remove(i));
        }
        for top in &mut self.items {
            if let Some(j) = top.items.iter().position(|s| s.id == id) {
                return Some(top.items.remove(j));
            }
        }
        None
    }

    fn list_mut(&mut self, parent: Option<&str>) -> Result<&mut Vec<Item>, String> {
        match parent {
            None => Ok(&mut self.items),
            Some(p) => self
                .items
                .iter_mut()
                .find(|i| i.id == p)
                .map(|i| &mut i.items)
                .ok_or_else(|| format!("\"{p}\" is not a top-level item")),
        }
    }

    /// The order `move_to` would give: `id` taken out and put in `parent`'s
    /// list at `index` (an index in that list without `id`). For the
    /// view's reorder by dragging or indenting.
    pub fn moved(&self, id: &str, parent: Option<&str>, index: usize) -> Vec<String> {
        let mut ids: Vec<String> = self.ids(parent).into_iter().filter(|i| i != id).collect();
        ids.insert(index.min(ids.len()), id.to_string());
        ids
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Toggle {
        item: String,
        done: bool,
    },
    /// `None` = unchanged; a note of `Some(None)` is cleared.
    Edit {
        item: String,
        label: Option<String>,
        note: Option<Option<String>>,
    },
    /// Appended to `parent`'s list (`None`: the top level).
    Add {
        item: Item,
        parent: Option<String>,
    },
    /// Removes its sub-items too.
    Remove {
        item: String,
    },
    /// `parent`'s list is now `items` (taken from wherever they are);
    /// items it held that aren't listed stay after them.
    Order {
        parent: Option<String>,
        items: Vec<String>,
    },
    /// A step's run status (the user skips one, or sets a failed one back
    /// to pending); done ⇄ pending is a [`Change::Toggle`].
    Status {
        item: String,
        status: Status,
    },
    /// The toolbar's "Run plan": turns the run plan on or off.
    RunMode {
        on: bool,
    },
    /// Run / Run next: the agent should do `item` (now running), then go
    /// on through the pending steps (`All`) or stop there (`Step`).
    Run {
        mode: Run,
        item: Option<String>,
    },
    /// Stop: don't start another step.
    Stop,
}

impl Editable for Checklist {
    type Change = Change;

    /// Run mode on/off, adds, orders (top level first), removes, edits,
    /// toggles and statuses, then run or stop.
    fn diff(old: &Self, new: &Self) -> Vec<Change> {
        let mut changes = Vec::new();
        let mut sim = old.clone();
        match (old.run, new.run) {
            (None, Some(Run::Idle)) => changes.push(Change::RunMode { on: true }),
            (Some(_), None) => changes.push(Change::RunMode { on: false }),
            _ => {}
        }
        for change in &changes {
            let _ = sim.apply(change);
        }
        // The step a run started: running now and not before.
        let started = new
            .flat()
            .into_iter()
            .map(|(_, item)| item)
            .find(|item| {
                item.status == Status::Running
                    && old
                        .item(&item.id)
                        .is_none_or(|o| o.status != Status::Running)
            })
            .map(|item| item.id.clone());
        let run = match new.run {
            Some(mode @ (Run::All | Run::Step)) if old.run != new.run || started.is_some() => {
                Some(Change::Run {
                    mode,
                    item: started.clone(),
                })
            }
            Some(Run::Idle) if matches!(old.run, Some(Run::All | Run::Step)) => Some(Change::Stop),
            _ => None,
        };
        let started = started.filter(|_| matches!(run, Some(Change::Run { .. })));
        for (parent, item) in new.flat() {
            if old.item(&item.id).is_none() {
                let change = Change::Add {
                    item: item.alone(),
                    parent: parent.map(|p| p.id.clone()),
                };
                let _ = sim.apply(&change);
                changes.push(change);
            }
        }
        let parents = std::iter::once(None).chain(new.items.iter().map(|i| Some(i.id.as_str())));
        for parent in parents {
            let wanted = new.ids(parent);
            let now: Vec<String> = sim
                .ids(parent)
                .into_iter()
                .filter(|id| new.place(id).is_some_and(|p| p.parent == parent))
                .collect();
            if now != wanted {
                let change = Change::Order {
                    parent: parent.map(str::to_string),
                    items: wanted,
                };
                let _ = sim.apply(&change);
                changes.push(change);
            }
        }
        for (parent, item) in old.flat() {
            let parent_gone = parent.is_some_and(|p| new.item(&p.id).is_none());
            if new.item(&item.id).is_none() && !(parent_gone && sim.item(&item.id).is_none()) {
                let change = Change::Remove {
                    item: item.id.clone(),
                };
                let _ = sim.apply(&change);
                changes.push(change);
            }
        }
        let mut toggles = Vec::new();
        for (_, item) in new.flat() {
            let Some(before) = sim.item(&item.id) else {
                continue;
            };
            if before.label != item.label || before.note != item.note {
                changes.push(Change::Edit {
                    item: item.id.clone(),
                    label: (before.label != item.label).then(|| item.label.clone()),
                    note: (before.note != item.note).then(|| item.note.clone()),
                });
            }
            if before.status != item.status {
                if before.status.plain() && item.status.plain() {
                    toggles.push(Change::Toggle {
                        item: item.id.clone(),
                        done: item.done,
                    });
                } else if started.as_ref() != Some(&item.id) {
                    toggles.push(Change::Status {
                        item: item.id.clone(),
                        status: item.status,
                    });
                }
            }
        }
        changes.append(&mut toggles);
        changes.extend(run);
        changes
    }

    fn apply(&mut self, change: &Change) -> Result<(), String> {
        let unknown = |id: &str| format!("unknown item \"{id}\"");
        match change {
            Change::Toggle { item, done } => {
                self.item_mut(item)
                    .ok_or_else(|| unknown(item))?
                    .set_status(if *done { Status::Done } else { Status::Pending });
            }
            Change::Status { item, status } => {
                self.item_mut(item)
                    .ok_or_else(|| unknown(item))?
                    .set_status(*status);
            }
            Change::RunMode { on } => {
                self.run = match (on, self.run) {
                    (true, None) => Some(Run::Idle),
                    (true, run) => run,
                    (false, _) => None,
                };
            }
            Change::Run { mode, item } => {
                if let Some(item) = item {
                    self.item_mut(item)
                        .ok_or_else(|| unknown(item))?
                        .set_status(Status::Running);
                }
                self.run = Some(*mode);
            }
            Change::Stop => {
                if self.run.is_some() {
                    self.run = Some(Run::Idle);
                }
            }
            Change::Edit { item, label, note } => {
                let it = self.item_mut(item).ok_or_else(|| unknown(item))?;
                if let Some(label) = label {
                    let label = label.trim();
                    if label.is_empty() {
                        return Err("an item needs a label".into());
                    }
                    it.label = truncate(label, MAX_ITEM_LABEL);
                }
                if let Some(note) = note {
                    it.note = note
                        .as_deref()
                        .map(str::trim)
                        .filter(|n| !n.is_empty())
                        .map(|n| truncate(n, MAX_TEXT));
                }
            }
            Change::Add { item, parent } => {
                if item.id.trim().is_empty() || item.label.trim().is_empty() {
                    return Err("a new item needs an id and a label".into());
                }
                if self.item(&item.id).is_some() {
                    return Err(format!("item \"{}\" already exists", item.id));
                }
                if self.count() >= MAX_ITEMS {
                    return Err(format!("too many items (at most {MAX_ITEMS})"));
                }
                if parent.is_some() && !item.items.is_empty() {
                    return Err("a sub-item can't have its own items".into());
                }
                self.list_mut(parent.as_deref())?.push(item.clone());
            }
            Change::Remove { item } => {
                self.take(item).ok_or_else(|| unknown(item))?;
            }
            Change::Order { parent, items } => {
                if let Some(missing) = items.iter().find(|id| self.item(id).is_none()) {
                    return Err(unknown(missing));
                }
                if let Some(p) = parent {
                    if items.contains(p) {
                        return Err(format!("\"{p}\" can't go under itself"));
                    }
                    self.list_mut(Some(p))?;
                    if let Some(nested) = items
                        .iter()
                        .filter_map(|id| self.item(id))
                        .find(|i| !i.items.is_empty())
                    {
                        return Err(format!(
                            "\"{}\" has sub-items, so it can't become one",
                            nested.label
                        ));
                    }
                }
                let mut next = self.clone();
                // Listed sub-items first, so a listed parent taken out
                // doesn't carry one of them away.
                let (subs, tops): (Vec<&String>, Vec<&String>) = items
                    .iter()
                    .partition(|id| next.place(id).is_some_and(|p| p.parent.is_some()));
                let mut found: Vec<Item> = subs
                    .into_iter()
                    .chain(tops)
                    .filter_map(|id| next.take(id))
                    .collect();
                let mut taken: Vec<Item> = items
                    .iter()
                    .filter_map(|id| {
                        let at = found.iter().position(|i| &i.id == id)?;
                        Some(found.remove(at))
                    })
                    .collect();
                let list = next.list_mut(parent.as_deref())?;
                let rest = std::mem::take(list);
                taken.extend(rest);
                *list = taken;
                *self = next;
            }
        }
        Ok(())
    }

    fn change_value(change: &Change) -> Value {
        match change {
            Change::Toggle { item, done } => json!({"op": "toggle", "item": item, "done": done}),
            Change::Edit { item, label, note } => {
                let mut map = Map::new();
                map.insert("op".into(), json!("edit"));
                map.insert("item".into(), json!(item));
                if let Some(label) = label {
                    map.insert("label".into(), json!(label));
                }
                if let Some(note) = note {
                    map.insert("note".into(), json!(note));
                }
                Value::Object(map)
            }
            Change::Add { item, parent } => {
                json!({"op": "add", "item": item.to_value(), "parent": parent})
            }
            Change::Remove { item } => json!({"op": "remove", "item": item}),
            Change::Order { parent, items } => {
                json!({"op": "order", "parent": parent, "items": items})
            }
            Change::Status { item, status } => {
                json!({"op": "status", "item": item, "status": status.name()})
            }
            Change::RunMode { on } => json!({"op": "run_mode", "on": on}),
            Change::Run { mode, item } => json!({"op": "run", "mode": mode.name(), "item": item}),
            Change::Stop => json!({"op": "stop"}),
        }
    }

    fn describe(change: &Change, before: &Self, after: &Self) -> String {
        let name = |id: &str| {
            after.item(id).or_else(|| before.item(id)).map_or_else(
                || id.to_string(),
                |i| format!("“{}”", truncate(&i.label, 60)),
            )
        };
        match change {
            Change::Toggle { item, done: true } => format!("checked {}", name(item)),
            Change::Toggle { item, done: false } => format!("unchecked {}", name(item)),
            Change::Edit { item, label, note } => {
                let mut parts = Vec::new();
                if let Some(label) = label {
                    let old = before.item(item).map_or(item.as_str(), |i| &i.label);
                    parts.push(format!("renamed “{old}” to “{label}”"));
                }
                match note {
                    Some(Some(_)) => parts.push(format!("updated the note on {}", name(item))),
                    Some(None) => parts.push(format!("cleared the note on {}", name(item))),
                    None => {}
                }
                parts.join(", ")
            }
            Change::Add { item, parent } => match parent {
                Some(p) => format!("added {} under {}", name(&item.id), name(p)),
                None => format!("added {}", name(&item.id)),
            },
            Change::Remove { item } => {
                let subs = before.item(item).map_or(0, |i| i.items.len());
                if subs == 0 {
                    format!("removed {}", name(item))
                } else {
                    format!(
                        "removed {} (and its {})",
                        name(item),
                        plural(subs, "sub-item")
                    )
                }
            }
            Change::Order { parent, items } => {
                let moved: Vec<String> = items
                    .iter()
                    .filter(|id| before.place(id).map(|p| p.parent) != Some(parent.as_deref()))
                    .map(|id| name(id))
                    .collect();
                let place = match parent {
                    Some(p) => format!("under {}", name(p)),
                    None => "to the top level".into(),
                };
                if !moved.is_empty() {
                    format!("moved {} {place}", list_names(&moved, 5))
                } else if let Some(p) = parent {
                    format!("reordered the items under {}", name(p))
                } else {
                    "reordered the list".into()
                }
            }
            Change::Status { item, status } => match status {
                Status::Skipped => format!("skipped {}", name(item)),
                Status::Pending => format!("set {} back to pending", name(item)),
                other => format!("marked {} as {}", name(item), other.name()),
            },
            Change::RunMode { on: true } => "turned on Run plan".into(),
            Change::RunMode { on: false } => "turned off Run plan".into(),
            Change::Run { mode, item } => {
                let step = item.as_deref().map(|id| {
                    let n = after.step_number(id).unwrap_or(0);
                    format!("step {n} {}", name(id))
                });
                match (mode, step) {
                    (Run::Step, Some(step)) => format!("asked the agent to do {step} next"),
                    (_, Some(step)) => format!("ran the plan from {step}"),
                    _ => "ran the plan".into(),
                }
            }
            Change::Stop => "stopped the plan".into(),
        }
    }

    fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert(
            "items".into(),
            self.items.iter().map(Item::to_value).collect(),
        );
        if let Some(run) = self.run {
            map.insert("run".into(), json!(run.name()));
        }
        if let Some(live) = &self.live {
            map.insert("live".into(), live.to_value());
        }
        Value::Object(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genui::edits::Edits;

    fn sample() -> Checklist {
        Checklist::parse(&json!({"items": [
            {"id": "plan", "label": "Plan", "done": true},
            {"id": "build", "label": "Build", "items": [
                {"id": "api", "label": "API"},
                {"id": "ui", "label": "UI", "note": "dark mode"}
            ]},
            "Ship it"
        ]}))
        .unwrap()
    }

    fn round_trip(old: &Checklist, new: &Checklist) {
        let mut replay = old.clone();
        for change in Checklist::diff(old, new) {
            replay
                .apply(&change)
                .unwrap_or_else(|e| panic!("{change:?}: {e}"));
        }
        assert_eq!(&replay, new);
    }

    #[test]
    fn parses_items_and_shorthands() {
        let list = sample();
        assert_eq!(list.count(), 5);
        assert_eq!(list.done(), 1);
        assert_eq!(list.items[2].id, "ship-it");
        assert_eq!(list.item("ui").unwrap().note.as_deref(), Some("dark mode"));
        assert_eq!(
            list.place("ui"),
            Some(Place {
                parent: Some("build"),
                index: 1
            })
        );
        assert_eq!(Checklist::parse(&list.to_value()).unwrap(), list);
    }

    #[test]
    fn readable_errors() {
        let errors = Checklist::parse(&json!({"items": [
            {"id": "a"}, {"id": "a"}, {"done": true},
            {"id": "b", "items": [{"id": "c", "items": ["deep"]}]}
        ]}))
        .unwrap_err();
        assert_eq!(
            errors,
            vec![
                "duplicate item id \"a\"",
                "item #3 has no label",
                "sub-items of \"b\" can't have their own items (one level only)",
            ]
        );
        let many: Vec<Value> = (0..=MAX_ITEMS).map(|i| json!(format!("t{i}"))).collect();
        assert_eq!(
            Checklist::parse(&json!({"items": many})).unwrap_err(),
            vec!["too many items (201, at most 200)"]
        );
    }

    #[test]
    fn edits_describe_and_round_trip() {
        let base = sample();
        let mut edits = Edits::new(base.clone());
        edits
            .edit(&Change::Toggle {
                item: "api".into(),
                done: true,
            })
            .unwrap();
        let new = Item::new(base.fresh_id("Write docs"), "Write docs");
        edits
            .edit(&Change::Add {
                item: new,
                parent: Some("build".into()),
            })
            .unwrap();
        // Drag "Ship it" to the top.
        let order = edits.working().moved("ship-it", None, 0);
        edits
            .edit(&Change::Order {
                parent: None,
                items: order,
            })
            .unwrap();
        edits
            .edit(&Change::Edit {
                item: "ui".into(),
                label: Some("Web UI".into()),
                note: Some(None),
            })
            .unwrap();
        edits
            .edit(&Change::Remove {
                item: "plan".into(),
            })
            .unwrap();
        let (_, lines) = edits.summarize();
        assert_eq!(
            lines,
            vec![
                "added “Write docs” under “Build”",
                "reordered the list",
                "removed “Plan”",
                "renamed “UI” to “Web UI”, cleared the note on “Web UI”",
                "checked “API”",
            ]
        );
        round_trip(&base, edits.working());
    }

    #[test]
    fn moving_between_levels_round_trips() {
        let base = sample();
        // Outdent "api" to the top level after "Build"; indent "Ship it" under "Plan".
        let mut list = base.clone();
        let order = list.moved("api", None, 2);
        list.apply(&Change::Order {
            parent: None,
            items: order,
        })
        .unwrap();
        list.apply(&Change::Order {
            parent: Some("plan".into()),
            items: vec!["ship-it".into()],
        })
        .unwrap();
        assert_eq!(list.ids(None), vec!["plan", "build", "api"]);
        assert_eq!(list.ids(Some("plan")), vec!["ship-it"]);
        assert_eq!(list.ids(Some("build")), vec!["ui"]);
        round_trip(&base, &list);
        let changes = Checklist::diff(&base, &list);
        let mut state = base.clone();
        let lines: Vec<String> = changes
            .iter()
            .map(|c| {
                let before = state.clone();
                state.apply(c).unwrap();
                Checklist::describe(c, &before, &state)
            })
            .collect();
        assert_eq!(
            lines,
            vec![
                "moved “API” to the top level",
                "moved “Ship it” under “Plan”"
            ]
        );
        // An item with sub-items can't become one.
        let refused = base.clone().apply(&Change::Order {
            parent: Some("plan".into()),
            items: vec!["build".into()],
        });
        assert_eq!(
            refused.unwrap_err(),
            "\"Build\" has sub-items, so it can't become one"
        );
    }

    #[test]
    fn removing_a_parent_after_moving_its_child_out_round_trips() {
        let base = sample();
        let mut list = base.clone();
        list.apply(&Change::Order {
            parent: None,
            items: vec!["ui".into()],
        })
        .unwrap();
        list.apply(&Change::Remove {
            item: "build".into(),
        })
        .unwrap();
        assert_eq!(list.ids(None), vec!["ui", "plan", "ship-it"]);
        round_trip(&base, &list);
    }

    #[test]
    fn items_keep_their_code_ref() {
        let list = Checklist::parse(&json!({"items": [
            {"id": "a", "label": "A", "ref": "src/a.rs:3"},
            {"id": "b", "label": "B", "items": [{"id": "c", "label": "C", "ref": {"path": "src/c.rs"}}]}
        ]}))
        .unwrap();
        assert_eq!(
            list.item("a").unwrap().code_ref.as_ref().unwrap().line,
            Some(3)
        );
        assert_eq!(
            list.item("c").unwrap().code_ref.as_ref().unwrap().path,
            "src/c.rs"
        );
        assert_eq!(Checklist::parse(&list.to_value()).unwrap(), list);
        assert_eq!(
            Checklist::parse(&json!({"items": [{"id": "a", "label": "A", "ref": "../x"}]}))
                .unwrap_err(),
            ["item \"a\": ref path \"../x\" escapes the workspace"]
        );
    }

    fn plan() -> Checklist {
        Checklist::parse(&json!({"run": true, "items": [
            {"id": "backup", "label": "Back up", "status": "done"},
            {"id": "migrate", "label": "Migrate", "items": [
                {"id": "users", "label": "users"},
                {"id": "orders", "label": "orders", "status": "failed", "note": "lock timeout"}
            ]},
            {"id": "verify", "label": "Verify"}
        ]}))
        .unwrap()
    }

    #[test]
    fn statuses_parse_with_done_and_round_trip() {
        let list = plan();
        assert_eq!(list.run, Some(Run::Idle));
        let backup = list.item("backup").unwrap();
        assert!(backup.done && backup.status == Status::Done);
        assert_eq!(list.item("orders").unwrap().status, Status::Failed);
        // `status` wins over `done`; an unknown one falls back to `done`.
        let odd = Checklist::parse(&json!({"items": [
            {"id": "a", "label": "A", "done": true, "status": "running"},
            {"id": "b", "label": "B", "done": true, "status": "later"}
        ]}))
        .unwrap();
        assert_eq!(odd.item("a").unwrap().status, Status::Running);
        assert!(!odd.item("a").unwrap().done);
        assert_eq!(odd.item("b").unwrap().status, Status::Done);
        // Done and pending stay `done` only on the wire.
        let value = list.to_value();
        assert_eq!(value["run"], "idle");
        assert_eq!(
            value["items"][0],
            json!({"id": "backup", "label": "Back up", "done": true})
        );
        assert_eq!(value["items"][1]["items"][1]["status"], "failed");
        assert_eq!(Checklist::parse(&value).unwrap(), list);
        assert_eq!(
            Checklist::parse(&json!({"items": [], "run": false}))
                .unwrap()
                .run,
            None
        );
    }

    #[test]
    fn steps_are_leaves_and_the_next_is_the_first_pending() {
        let list = plan();
        let steps: Vec<&str> = list.steps().iter().map(|i| i.id.as_str()).collect();
        assert_eq!(steps, vec!["backup", "users", "orders", "verify"]);
        assert_eq!(list.next_step().map(|i| i.id.as_str()), Some("users"));
        assert_eq!(list.step_number("verify"), Some(4));
        assert_eq!(list.running(), None);
    }

    #[test]
    fn run_skip_and_stop_are_changes_the_agent_reads() {
        let base = plan();
        let mut edits = Edits::new(base.clone());
        // Skip "users", then Run next: the next pending step is "verify".
        edits
            .edit(&Change::Status {
                item: "users".into(),
                status: Status::Skipped,
            })
            .unwrap();
        let next = edits.working().next_step().unwrap().id.clone();
        assert_eq!(next, "verify");
        edits
            .edit(&Change::Run {
                mode: Run::Step,
                item: Some(next),
            })
            .unwrap();
        let working = edits.working();
        assert_eq!(working.run, Some(Run::Step));
        assert_eq!(working.running().map(|i| i.id.as_str()), Some("verify"));
        let (values, lines) = edits.summarize();
        assert_eq!(
            values,
            vec![
                json!({"op": "status", "item": "users", "status": "skipped"}),
                json!({"op": "run", "mode": "step", "item": "verify"}),
            ]
        );
        assert_eq!(
            lines,
            vec![
                "skipped “users”",
                "asked the agent to do step 4 “Verify” next"
            ]
        );
        round_trip(&base, edits.working());

        // Stop while a step runs: back to idle, the step stays the agent's.
        let running = edits.working().clone();
        let mut stopped = running.clone();
        stopped.apply(&Change::Stop).unwrap();
        assert_eq!(stopped.run, Some(Run::Idle));
        assert_eq!(Checklist::diff(&running, &stopped), vec![Change::Stop]);

        // Turning the plan on and off.
        let mut plain = sample();
        let before = plain.clone();
        plain.apply(&Change::RunMode { on: true }).unwrap();
        assert_eq!(
            Checklist::diff(&before, &plain),
            vec![Change::RunMode { on: true }]
        );
        plain.apply(&Change::RunMode { on: false }).unwrap();
        assert_eq!(plain, before);
    }

    #[test]
    fn a_run_settles_when_its_steps_are_through() {
        let run = |mode: &str, statuses: [&str; 2]| {
            Checklist::parse(&json!({"run": mode, "items": [
                {"id": "a", "label": "A", "status": statuses[0]},
                {"id": "b", "label": "B", "status": statuses[1]}
            ]}))
            .unwrap()
            .run
        };
        assert_eq!(run("step", ["running", "pending"]), Some(Run::Step));
        assert_eq!(
            run("step", ["done", "pending"]),
            Some(Run::Idle),
            "one step done"
        );
        assert_eq!(
            run("all", ["done", "pending"]),
            Some(Run::All),
            "more to do"
        );
        assert_eq!(
            run("all", ["done", "failed"]),
            Some(Run::Idle),
            "nothing pending"
        );
        assert_eq!(run("all", ["done", "running"]), Some(Run::All));
    }
}
