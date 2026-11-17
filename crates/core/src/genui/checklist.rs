//! The checklist kind: items with one level of sub-items that the user
//! checks off, adds, edits, removes and reorders (plans, TODOs).

use serde_json::{Map, Value, json};

use super::diagram::{Errors, MAX_TEXT, array, id_of, text, truncate};
use super::edits::{Editable, fresh_id, list_names, plural};

pub const MAX_ITEMS: usize = 200;
pub const MAX_ITEM_LABEL: usize = 200;

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: String,
    pub label: String,
    pub done: bool,
    pub note: Option<String>,
    /// Sub-items (only on top-level items: one level).
    pub items: Vec<Item>,
}

impl Item {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            done: false,
            note: None,
            items: Vec::new(),
        }
    }

    fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("id".into(), json!(self.id));
        map.insert("label".into(), json!(self.label));
        map.insert("done".into(), json!(self.done));
        if let Some(note) = &self.note {
            map.insert("note".into(), json!(note));
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
            Some(Item {
                id,
                label,
                done: raw.get("done").and_then(Value::as_bool) == Some(true),
                note: text(raw.get("note"), MAX_TEXT),
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
        if !errors.0.is_empty() {
            return Err(errors.0);
        }
        Ok(list)
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
}

impl Editable for Checklist {
    type Change = Change;

    /// Adds, orders (top level first), removes, edits, toggles.
    fn diff(old: &Self, new: &Self) -> Vec<Change> {
        let mut changes = Vec::new();
        let mut sim = old.clone();
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
            if before.done != item.done {
                toggles.push(Change::Toggle {
                    item: item.id.clone(),
                    done: item.done,
                });
            }
        }
        changes.append(&mut toggles);
        changes
    }

    fn apply(&mut self, change: &Change) -> Result<(), String> {
        let unknown = |id: &str| format!("unknown item \"{id}\"");
        match change {
            Change::Toggle { item, done } => {
                self.item_mut(item).ok_or_else(|| unknown(item))?.done = *done;
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
        }
    }

    fn to_value(&self) -> Value {
        json!({"items": self.items.iter().map(Item::to_value).collect::<Vec<_>>()})
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
}
