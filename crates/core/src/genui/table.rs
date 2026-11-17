//! The table kind: typed columns and rows the user sorts, filters, selects
//! and (when the agent allows it) edits. Selection and cell edits go back
//! as changes; sorting and filtering only change the view.

use std::cmp::Ordering;

use serde_json::{Map, Value, json};

use super::diagram::{Errors, MAX_LABEL, array, id_of, text, truncate};
use super::edits::{Editable, format_number, list_names, number, plural};
use super::live::Live;

pub const MAX_COLUMNS: usize = 30;
pub const MAX_ROWS: usize = 500;
/// A cell's text is cut to this many characters.
pub const MAX_CELL: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColumnType {
    #[default]
    Text,
    Number,
    Date,
    Status,
    Link,
}

impl ColumnType {
    pub const ALL: [Self; 5] = [
        Self::Text,
        Self::Number,
        Self::Date,
        Self::Status,
        Self::Link,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Number => "number",
            Self::Date => "date",
            Self::Status => "status",
            Self::Link => "link",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|t| t.name().eq_ignore_ascii_case(s.trim()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

impl Align {
    pub fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        [Self::Left, Self::Center, Self::Right]
            .into_iter()
            .find(|a| a.name().eq_ignore_ascii_case(s.trim()))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub key: String,
    pub label: String,
    pub kind: ColumnType,
    pub align: Align,
}

/// A row's cells by column key: strings, numbers, booleans, or a link
/// object `{text, url}`. A missing key is an empty cell.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: String,
    pub cells: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sort {
    pub column: String,
    pub descending: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub columns: Vec<Column>,
    pub rows: Vec<Row>,
    pub sort: Option<Sort>,
    /// The user can select rows (default true).
    pub selectable: bool,
    /// The user can edit cells (default false).
    pub editable: bool,
    /// Selected row ids, in table order.
    pub selection: Vec<String>,
    /// Rows read from a live source (see `genui::live`).
    pub live: Option<Box<Live>>,
}

/// How a status cell is colored, from its words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Success,
    Danger,
    Warning,
    Info,
    Muted,
}

/// A status cell's tone from the start of its words ("Failing" → danger,
/// "in progress" → info); unknown words are muted.
pub fn status_tone(text: &str) -> Tone {
    let t = text.to_lowercase();
    let words: Vec<&str> = t.split(|c: char| !c.is_alphanumeric()).collect();
    let has = |stems: &[&str]| words.iter().any(|w| stems.iter().any(|s| w.starts_with(s)));
    if has(&[
        "fail", "error", "block", "broken", "risk", "critical", "reject", "down",
    ]) {
        Tone::Danger
    } else if has(&[
        "done", "ok", "pass", "success", "ready", "complete", "merged", "live", "approved",
        "active", "healthy",
    ]) {
        Tone::Success
    } else if has(&[
        "warn", "pending", "wait", "review", "slow", "degraded", "todo",
    ]) {
        Tone::Warning
    } else if has(&["progress", "running", "doing", "new", "open", "planned"]) {
        Tone::Info
    } else {
        Tone::Muted
    }
}

/// What a cell shows: numbers without a trailing ".0", a link's text.
pub fn cell_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.as_f64().map_or_else(|| n.to_string(), format_number),
        Value::Bool(true) => "yes".into(),
        Value::Bool(false) => "no".into(),
        Value::Object(link) => link
            .get("text")
            .or_else(|| link.get("url"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        Value::Array(_) => String::new(),
    }
}

/// A link cell's URL: the string itself or the object's `url`.
pub fn link_url(value: &Value) -> Option<&str> {
    match value {
        Value::String(s) => Some(s.as_str()),
        Value::Object(link) => link.get("url").and_then(Value::as_str),
        _ => None,
    }
    .filter(|url| !url.is_empty())
}

/// A cell value as stored: text cut to [`MAX_CELL`], numbers kept, a link
/// object reduced to its text and url. `None` for values a cell can't hold.
fn clean_cell(value: &Value) -> Option<Value> {
    Some(match value {
        Value::Null | Value::Bool(_) => value.clone(),
        Value::Number(n) if n.as_f64().is_some_and(f64::is_finite) => value.clone(),
        Value::String(s) => json!(truncate(s.trim(), MAX_CELL)),
        Value::Object(link) => {
            let url = link.get("url").and_then(Value::as_str)?;
            let mut out = Map::new();
            if let Some(text) = text(link.get("text"), MAX_CELL) {
                out.insert("text".into(), json!(text));
            }
            out.insert("url".into(), json!(truncate(url, MAX_CELL)));
            Value::Object(out)
        }
        _ => return None,
    })
}

impl Table {
    pub fn parse(spec: &Value) -> Result<Table, Vec<String>> {
        let Some(spec) = spec.as_object() else {
            return Err(vec!["the spec must be a JSON object".into()]);
        };
        let mut errors = Errors::default();
        let Some(columns_raw) = spec.get("columns").and_then(Value::as_array) else {
            return Err(vec!["\"columns\" must be an array".into()]);
        };
        let rows_raw = match spec.get("rows") {
            None | Some(Value::Null) => &[][..],
            Some(Value::Array(rows)) => rows.as_slice(),
            Some(_) => return Err(vec!["\"rows\" must be an array".into()]),
        };
        if columns_raw.is_empty() {
            errors.push("a table needs at least one column".into());
        }
        if columns_raw.len() > MAX_COLUMNS {
            errors.push(format!(
                "too many columns ({}, at most {MAX_COLUMNS})",
                columns_raw.len()
            ));
        }
        if rows_raw.len() > MAX_ROWS {
            errors.push(format!(
                "too many rows ({}, at most {MAX_ROWS})",
                rows_raw.len()
            ));
        }

        let mut columns: Vec<Column> = Vec::new();
        for (i, raw) in columns_raw.iter().enumerate() {
            // "columns": ["name", "status"] is a shorthand for text columns.
            let key = match raw {
                Value::String(_) | Value::Number(_) => id_of(raw),
                _ => raw.get("key").or_else(|| raw.get("id")).and_then(id_of),
            };
            let Some(key) = key else {
                errors.push(format!("column #{} has no key", i + 1));
                continue;
            };
            if columns.iter().any(|c| c.key == key) {
                errors.push(format!("duplicate column key \"{key}\""));
                continue;
            }
            let kind = raw
                .get("type")
                .and_then(Value::as_str)
                .and_then(ColumnType::parse)
                .unwrap_or_default();
            columns.push(Column {
                label: text(raw.get("label"), MAX_LABEL).unwrap_or_else(|| key.clone()),
                align: raw
                    .get("align")
                    .and_then(Value::as_str)
                    .and_then(Align::parse)
                    .unwrap_or(if kind == ColumnType::Number {
                        Align::Right
                    } else {
                        Align::Left
                    }),
                kind,
                key,
            });
        }

        let mut rows: Vec<Row> = Vec::new();
        for (i, raw) in rows_raw.iter().enumerate().take(MAX_ROWS) {
            let id = raw
                .get("id")
                .and_then(id_of)
                .unwrap_or_else(|| format!("row-{}", i + 1));
            if rows.iter().any(|r| r.id == id) {
                errors.push(format!("duplicate row id \"{id}\""));
                continue;
            }
            let mut cells = Map::new();
            match raw.get("cells") {
                // Positional: one value per column, in order.
                Some(Value::Array(values)) => {
                    for (column, value) in columns.iter().zip(values) {
                        if let Some(value) = clean_cell(value) {
                            cells.insert(column.key.clone(), value);
                        }
                    }
                }
                Some(Value::Object(map)) => {
                    for column in &columns {
                        if let Some(value) = map.get(&column.key).and_then(clean_cell) {
                            cells.insert(column.key.clone(), value);
                        }
                    }
                }
                // The row's own keys are its cells: {"id": "a", "name": "Ada"}.
                None if raw.is_object() => {
                    for column in &columns {
                        if let Some(value) = raw.get(&column.key).and_then(clean_cell) {
                            cells.insert(column.key.clone(), value);
                        }
                    }
                }
                _ => {
                    errors.push(format!(
                        "row \"{id}\" must be an object whose \"cells\" is an object or an array"
                    ));
                    continue;
                }
            }
            cells.retain(|_, v| !v.is_null());
            rows.push(Row { id, cells });
        }

        let sort = match spec.get("sort") {
            None | Some(Value::Null) => None,
            Some(raw) => {
                let column = raw
                    .get("column")
                    .or_else(|| raw.get("key"))
                    .and_then(id_of)
                    .or_else(|| id_of(raw));
                match column {
                    Some(column) if columns.iter().any(|c| c.key == column) => Some(Sort {
                        column,
                        descending: raw
                            .get("direction")
                            .and_then(Value::as_str)
                            .is_some_and(|d| d.eq_ignore_ascii_case("desc")),
                    }),
                    Some(column) => {
                        errors.push(format!("sort column \"{column}\" is not a column"));
                        None
                    }
                    None => {
                        errors.push("\"sort\" needs a column".into());
                        None
                    }
                }
            }
        };
        let selectable = spec.get("selectable").and_then(Value::as_bool) != Some(false);
        let wanted: Vec<String> = array(spec.get("selection"))
            .iter()
            .filter_map(id_of)
            .collect();
        let selection = if selectable {
            rows.iter()
                .filter(|r| wanted.contains(&r.id))
                .map(|r| r.id.clone())
                .collect()
        } else {
            Vec::new()
        };

        let live = match spec.get("live") {
            None | Some(Value::Null) => None,
            Some(raw) => match Live::parse(raw) {
                Ok(live) => {
                    let keys: Vec<&str> = columns.iter().map(|c| c.key.as_str()).collect();
                    live.check_columns(&keys)
                        .into_iter()
                        .for_each(|e| errors.push(e));
                    Some(Box::new(live))
                }
                Err(live_errors) => {
                    live_errors.into_iter().for_each(|e| errors.push(e));
                    None
                }
            },
        };
        if !errors.0.is_empty() {
            return Err(errors.0);
        }
        Ok(Table {
            columns,
            rows,
            sort,
            selectable,
            editable: spec.get("editable").and_then(Value::as_bool) == Some(true),
            selection,
            live,
        })
    }

    pub fn column(&self, key: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.key == key)
    }

    pub fn row(&self, id: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.id == id)
    }

    pub fn cell(&self, row: &str, column: &str) -> &Value {
        self.row(row)
            .and_then(|r| r.cells.get(column))
            .unwrap_or(&Value::Null)
    }

    /// A row's name for people: its first column's text, else its id.
    pub fn row_label(&self, id: &str) -> String {
        let first = self
            .columns
            .first()
            .map(|c| cell_text(self.cell(id, &c.key)))
            .unwrap_or_default();
        if first.trim().is_empty() {
            id.to_string()
        } else {
            truncate(&first, 40)
        }
    }

    /// Row indexes to show: those matching `filter` (any cell contains it,
    /// ignoring case), ordered by `sort` (stable; empty cells last).
    pub fn visible_rows(&self, sort: Option<&Sort>, filter: &str) -> Vec<usize> {
        let needle = filter.trim().to_lowercase();
        let mut rows: Vec<usize> = (0..self.rows.len())
            .filter(|&ix| {
                needle.is_empty()
                    || self.rows[ix]
                        .cells
                        .values()
                        .any(|v| cell_text(v).to_lowercase().contains(&needle))
            })
            .collect();
        if let Some(sort) = sort
            && let Some(column) = self.column(&sort.column)
        {
            let key = &column.key;
            rows.sort_by(|&a, &b| {
                let (a, b) = (
                    self.rows[a].cells.get(key).unwrap_or(&Value::Null),
                    self.rows[b].cells.get(key).unwrap_or(&Value::Null),
                );
                match (a.is_null(), b.is_null()) {
                    (true, true) => Ordering::Equal,
                    (true, false) => Ordering::Greater,
                    (false, true) => Ordering::Less,
                    _ => {
                        let order = compare(column.kind, a, b);
                        if sort.descending {
                            order.reverse()
                        } else {
                            order
                        }
                    }
                }
            });
        }
        rows
    }

    /// A user's text for `column` as the value stored: numbers for number
    /// columns (an error for anything else), empty text clears the cell.
    pub fn value_from_text(&self, column: &str, text: &str) -> Result<Value, String> {
        let column = self
            .column(column)
            .ok_or_else(|| format!("unknown column \"{column}\""))?;
        let text = text.trim();
        if text.is_empty() {
            return Ok(Value::Null);
        }
        match column.kind {
            ColumnType::Number => number(&json!(text))
                .map(|n| json!(n))
                .ok_or_else(|| format!("{} needs a number, not “{text}”", column.label)),
            _ => Ok(json!(truncate(text, MAX_CELL))),
        }
    }
}

fn compare(kind: ColumnType, a: &Value, b: &Value) -> Ordering {
    if kind == ColumnType::Number
        && let (Some(a), Some(b)) = (number(a), number(b))
    {
        return a.total_cmp(&b);
    }
    // Dates as ISO text sort as text.
    cell_text(a)
        .to_lowercase()
        .cmp(&cell_text(b).to_lowercase())
}

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// The selected rows now (replaces the selection).
    Select { rows: Vec<String> },
    /// A cell's new value (`null` clears it).
    EditCell {
        row: String,
        column: String,
        value: Value,
    },
}

impl Editable for Table {
    type Change = Change;

    /// Cell edits in row order, then the selection.
    fn diff(old: &Self, new: &Self) -> Vec<Change> {
        let mut changes = Vec::new();
        for row in &new.rows {
            let Some(before) = old.row(&row.id) else {
                continue;
            };
            for column in &new.columns {
                let value = row.cells.get(&column.key).unwrap_or(&Value::Null);
                if before.cells.get(&column.key).unwrap_or(&Value::Null) != value {
                    changes.push(Change::EditCell {
                        row: row.id.clone(),
                        column: column.key.clone(),
                        value: value.clone(),
                    });
                }
            }
        }
        if old.selection != new.selection {
            changes.push(Change::Select {
                rows: new.selection.clone(),
            });
        }
        changes
    }

    fn apply(&mut self, change: &Change) -> Result<(), String> {
        match change {
            Change::Select { rows } => {
                if !self.selectable {
                    return Err("this table's rows can't be selected".into());
                }
                if let Some(unknown) = rows.iter().find(|id| self.row(id).is_none()) {
                    return Err(format!("unknown row \"{unknown}\""));
                }
                self.selection = self
                    .rows
                    .iter()
                    .filter(|r| rows.contains(&r.id))
                    .map(|r| r.id.clone())
                    .collect();
            }
            Change::EditCell { row, column, value } => {
                if !self.editable {
                    return Err("this table is read-only".into());
                }
                let kind = self
                    .column(column)
                    .ok_or_else(|| format!("unknown column \"{column}\""))?
                    .kind;
                if kind == ColumnType::Number && !value.is_null() && !value.is_number() {
                    return Err(format!("\"{column}\" holds numbers"));
                }
                let value = clean_cell(value).ok_or("a cell holds text, a number or a link")?;
                let r = self
                    .rows
                    .iter_mut()
                    .find(|r| &r.id == row)
                    .ok_or_else(|| format!("unknown row \"{row}\""))?;
                if value.is_null() {
                    r.cells.remove(column);
                } else {
                    r.cells.insert(column.clone(), value);
                }
            }
        }
        Ok(())
    }

    fn change_value(change: &Change) -> Value {
        match change {
            Change::Select { rows } => json!({"op": "select", "rows": rows}),
            Change::EditCell { row, column, value } => {
                json!({"op": "edit_cell", "row": row, "column": column, "value": value})
            }
        }
    }

    fn describe(change: &Change, before: &Self, after: &Self) -> String {
        match change {
            Change::Select { rows } if rows.is_empty() => "cleared the selection".into(),
            Change::Select { rows } => {
                let names: Vec<String> = rows.iter().map(|id| after.row_label(id)).collect();
                format!(
                    "selected {}: {}",
                    plural(rows.len(), "row"),
                    list_names(&names, 8)
                )
            }
            Change::EditCell { row, column, value } => {
                let label = after.column(column).map_or(column.as_str(), |c| &c.label);
                let old = cell_text(before.cell(row, column));
                let new = cell_text(value);
                let name = before.row_label(row);
                match (old.is_empty(), new.is_empty()) {
                    (_, true) => format!("cleared {label} of {name} (was “{old}”)"),
                    (true, false) => format!("set {label} of {name} to “{new}”"),
                    _ => format!("changed {label} of {name} from “{old}” to “{new}”"),
                }
            }
        }
    }

    fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert(
            "columns".into(),
            self.columns
                .iter()
                .map(|c| {
                    json!({"key": c.key, "label": c.label, "type": c.kind.name(), "align": c.align.name()})
                })
                .collect(),
        );
        map.insert(
            "rows".into(),
            self.rows
                .iter()
                .map(|r| json!({"id": r.id, "cells": r.cells}))
                .collect(),
        );
        if let Some(sort) = &self.sort {
            map.insert(
                "sort".into(),
                json!({"column": sort.column, "direction": if sort.descending { "desc" } else { "asc" }}),
            );
        }
        map.insert("selectable".into(), json!(self.selectable));
        map.insert("editable".into(), json!(self.editable));
        map.insert("selection".into(), json!(self.selection));
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

    fn sample() -> Table {
        Table::parse(&json!({
            "columns": [
                {"key": "name", "label": "Name"},
                {"key": "tests", "label": "Tests", "type": "number"},
                {"key": "state", "label": "State", "type": "status"},
                {"key": "pr", "label": "PR", "type": "link"}
            ],
            "rows": [
                {"id": "api", "cells": {"name": "API", "tests": 120, "state": "passing", "pr": {"text": "#12", "url": "https://x/12"}}},
                {"id": "web", "cells": ["Web", 8, "failing"]},
                {"id": "db", "name": "DB", "state": "pending"}
            ],
            "editable": true,
            "selection": ["web", "nope"]
        }))
        .unwrap()
    }

    #[test]
    fn parses_cells_in_every_shape_with_defaults() {
        let t = sample();
        assert_eq!(t.columns[1].align, Align::Right);
        assert_eq!(t.columns[0].align, Align::Left);
        assert_eq!(t.cell("web", "tests"), &json!(8));
        assert_eq!(t.cell("db", "name"), &json!("DB"));
        assert_eq!(t.cell("db", "tests"), &Value::Null);
        assert_eq!(cell_text(t.cell("api", "pr")), "#12");
        assert_eq!(link_url(t.cell("api", "pr")), Some("https://x/12"));
        assert_eq!(t.selection, vec!["web".to_string()]);
        assert!(t.selectable && t.editable);
        // Columns as plain strings.
        let short =
            Table::parse(&json!({"columns": ["a", "b"], "rows": [{"cells": [1, 2]}]})).unwrap();
        assert_eq!(short.rows[0].id, "row-1");
        assert_eq!(short.columns[1].label, "b");
    }

    #[test]
    fn readable_errors() {
        assert_eq!(
            Table::parse(&json!({"rows": []})).unwrap_err(),
            vec!["\"columns\" must be an array"]
        );
        let errors = Table::parse(&json!({
            "columns": [{"key": "a"}, {"key": "a"}, {"label": "x"}],
            "rows": [{"id": "r"}, {"id": "r"}, 3],
            "sort": {"column": "zzz"}
        }))
        .unwrap_err();
        assert_eq!(
            errors,
            vec![
                "duplicate column key \"a\"",
                "column #3 has no key",
                "duplicate row id \"r\"",
                "row \"row-3\" must be an object whose \"cells\" is an object or an array",
                "sort column \"zzz\" is not a column",
            ]
        );
        let rows: Vec<Value> = (0..=MAX_ROWS).map(|i| json!({"id": i})).collect();
        let errors = Table::parse(&json!({"columns": ["a"], "rows": rows})).unwrap_err();
        assert_eq!(
            errors,
            vec![format!("too many rows (501, at most {MAX_ROWS})")]
        );
    }

    #[test]
    fn sorts_numbers_numerically_and_filters_any_cell() {
        let t = sample();
        let by = |column: &str, descending| Sort {
            column: column.into(),
            descending,
        };
        assert_eq!(t.visible_rows(Some(&by("tests", false)), ""), vec![1, 0, 2]);
        assert_eq!(t.visible_rows(Some(&by("tests", true)), ""), vec![0, 1, 2]);
        assert_eq!(t.visible_rows(Some(&by("name", true)), ""), vec![1, 2, 0]);
        assert_eq!(t.visible_rows(None, "FAIL"), vec![1]);
        assert_eq!(t.visible_rows(None, "#12"), vec![0]);
    }

    #[test]
    fn edits_round_trip_and_describe() {
        let base = sample();
        let mut edits = Edits::new(base.clone());
        let tests = base.value_from_text("tests", "1,300").unwrap();
        assert_eq!(tests, json!(1300.0));
        assert!(base.value_from_text("tests", "lots").is_err());
        edits
            .edit(&Change::EditCell {
                row: "api".into(),
                column: "tests".into(),
                value: tests,
            })
            .unwrap();
        edits
            .edit(&Change::EditCell {
                row: "db".into(),
                column: "state".into(),
                value: Value::Null,
            })
            .unwrap();
        edits
            .edit(&Change::Select {
                rows: vec!["db".into(), "api".into()],
            })
            .unwrap();
        assert_eq!(edits.working().selection, vec!["api", "db"]);
        let (values, lines) = edits.summarize();
        assert_eq!(
            lines,
            vec![
                "changed Tests of API from “120” to “1300”",
                "cleared State of DB (was “pending”)",
                "selected 2 rows: API and DB",
            ]
        );
        assert_eq!(values[2], json!({"op": "select", "rows": ["api", "db"]}));
        // The wire spec parses back to the same table.
        assert_eq!(
            Table::parse(&edits.working().to_value()).unwrap(),
            *edits.working()
        );
        let mut replay = base.clone();
        for change in Table::diff(&base, edits.working()) {
            replay.apply(&change).unwrap();
        }
        assert_eq!(&replay, edits.working());
    }

    #[test]
    fn read_only_tables_refuse_edits() {
        let mut t =
            Table::parse(&json!({"columns": ["a"], "rows": [{"id": "r"}], "selectable": false}))
                .unwrap();
        let edit = Change::EditCell {
            row: "r".into(),
            column: "a".into(),
            value: json!("x"),
        };
        assert_eq!(t.apply(&edit).unwrap_err(), "this table is read-only");
        assert!(t.apply(&Change::Select { rows: vec![] }).is_err());
    }

    #[test]
    fn status_tones() {
        assert_eq!(status_tone("Passing ✓"), Tone::Success);
        assert_eq!(status_tone("Build failed"), Tone::Danger);
        assert_eq!(status_tone("in progress"), Tone::Info);
        assert_eq!(status_tone("pending review"), Tone::Warning);
        assert_eq!(status_tone("unknown"), Tone::Muted);
    }
}
