//! Exporting a surface as a file people keep: a diagram as Mermaid or SVG,
//! a table as CSV or Markdown, checklists and boards as Markdown, a chart
//! as an image (an SVG the app rasterizes to PNG) or CSV of its data, a
//! form's values as JSON and an html page as itself.

use std::fmt::Write as _;

use serde_json::{Map, Value, json};

use super::chart::{Axes, Chart, ChartType, Geometry, Shape, short_number};
use super::checklist::Checklist;
use super::diagram::{Diagram, EdgeStyle, NodeKind, Status};
use super::form::Form;
use super::html::HtmlSpec;
use super::kanban::Board;
use super::layout::{self, GROUP_HEADER, Point, Rect};
use super::refs::CodeRef;
use super::table::{Align, ColumnType, Table, cell_text, link_url};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Mermaid,
    Svg,
    Csv,
    Markdown,
    /// [`render`] gives the SVG to rasterize.
    Png,
    Json,
    Html,
}

impl Format {
    /// What menus call it.
    pub fn name(self) -> &'static str {
        match self {
            Format::Mermaid => "Mermaid",
            Format::Svg => "SVG",
            Format::Csv => "CSV",
            Format::Markdown => "Markdown",
            Format::Png => "PNG",
            Format::Json => "JSON",
            Format::Html => "HTML",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Mermaid => "mmd",
            Format::Svg => "svg",
            Format::Csv => "csv",
            Format::Markdown => "md",
            Format::Png => "png",
            Format::Json => "json",
            Format::Html => "html",
        }
    }

    /// The fence a Markdown document wraps it in (`None`: it is Markdown).
    pub fn fence(self) -> Option<&'static str> {
        match self {
            Format::Mermaid => Some("mermaid"),
            Format::Svg | Format::Html => Some("html"),
            Format::Csv => Some("csv"),
            Format::Json => Some("json"),
            Format::Markdown | Format::Png => None,
        }
    }
}

/// The formats a kind exports to, the one for docs first.
pub fn formats(kind: &str) -> &'static [Format] {
    match kind {
        "diagram" => &[Format::Mermaid, Format::Svg],
        "table" => &[Format::Markdown, Format::Csv],
        "checklist" | "kanban" => &[Format::Markdown],
        "chart" => &[Format::Png, Format::Csv],
        "form" => &[Format::Json],
        "html" => &[Format::Html],
        _ => &[],
    }
}

/// The text format a kind goes into docs as.
pub fn docs_format(kind: &str) -> Option<Format> {
    formats(kind)
        .iter()
        .copied()
        .find(|f| *f != Format::Png && *f != Format::Svg)
}

/// Colors (CSS colors) and the font of drawn exports.
#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub background: String,
    pub surface: String,
    pub text: String,
    pub muted: String,
    pub border: String,
    pub accent: String,
    /// One per series or node kind tone, cycled.
    pub series: Vec<String>,
    pub font: String,
}

impl Default for Palette {
    fn default() -> Self {
        Palette {
            background: "#ffffff".into(),
            surface: "#f6f6f8".into(),
            text: "#1c1c22".into(),
            muted: "#6b6b76".into(),
            border: "#d4d4dc".into(),
            accent: "#d4577c".into(),
            series: [
                "#3b82f6", "#d4577c", "#16a34a", "#d97706", "#b08d57", "#c084fc", "#dc2626",
                "#6b7280",
            ]
            .map(String::from)
            .to_vec(),
            font: "sans-serif".into(),
        }
    }
}

/// `title` as a file name: a slug plus the format's extension.
pub fn file_name(title: &str, format: Format) -> String {
    let mut slug = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_end_matches('-').chars().take(60).collect();
    let slug = if slug.is_empty() { "ui".into() } else { slug };
    format!("{slug}.{}", format.extension())
}

/// A surface's `spec` of `kind` as `format` (an SVG for [`Format::Png`]).
pub fn render(
    kind: &str,
    title: &str,
    spec: &Value,
    format: Format,
    palette: &Palette,
) -> Result<String, String> {
    if !formats(kind).contains(&format) {
        return Err(format!("a {kind} can't be exported as {}", format.name()));
    }
    let errors = |e: Vec<String>| e.join("; ");
    Ok(match (kind, format) {
        ("diagram", Format::Mermaid) => mermaid(&Diagram::parse(spec).map_err(errors)?, title),
        ("diagram", Format::Svg) => diagram_svg(&Diagram::parse(spec).map_err(errors)?, palette),
        ("table", Format::Csv) => table_csv(&Table::parse(spec).map_err(errors)?),
        ("table", Format::Markdown) => table_markdown(&Table::parse(spec).map_err(errors)?),
        ("checklist", _) => checklist_markdown(&Checklist::parse(spec).map_err(errors)?),
        ("kanban", _) => kanban_markdown(&Board::parse(spec).map_err(errors)?),
        ("chart", Format::Csv) => chart_csv(&Chart::parse(spec).map_err(errors)?),
        ("chart", _) => chart_svg(&Chart::parse(spec).map_err(errors)?, title, palette),
        ("form", _) => form_json(&Form::parse(spec).map_err(errors)?),
        ("html", _) => HtmlSpec::parse(spec).map_err(errors)?.html,
        _ => unreachable!("checked against formats()"),
    })
}

/// Exports longer than this (bytes) aren't put in the docs request: the
/// agent has the spec already.
pub const MAX_DOCS_TEXT: usize = 60_000;

/// `/gentle-ui:event {"op":"docs", id, format, fence, text}`: the user
/// asks the agent to add the surface to the project's docs, in its
/// [`docs_format`] (the text left out when it's over [`MAX_DOCS_TEXT`]).
/// It carries no version, so the extension answers it as version 0, which
/// no edit being sent waits for.
pub fn docs_command(id: &str, kind: &str, title: &str, spec: &Value) -> Result<String, String> {
    let format = docs_format(kind).ok_or_else(|| format!("a {kind} has no docs format"))?;
    let text = render(kind, title, spec, format, &Palette::default())?;
    let text = (text.len() <= MAX_DOCS_TEXT).then_some(text);
    let event = json!({
        "op": "docs",
        "id": id,
        "format": format.name(),
        "fence": format.fence(),
        "text": text,
    });
    Ok(format!("/{} {event}", super::EVENT_COMMAND))
}

// ── Mermaid ────────────────────────────────────────────────────────────────

/// A Mermaid `flowchart` id for each of `ids`: letters, digits and `_`,
/// prefixed so none is a keyword (`end`, `graph`), unique.
fn mermaid_ids<'a>(prefix: &str, ids: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        let clean: String = id
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let base = format!("{prefix}{clean}");
        let mut candidate = base.clone();
        let mut n = 2;
        while out.contains(&candidate) {
            candidate = format!("{base}_{n}");
            n += 1;
        }
        out.push(candidate);
    }
    out
}

/// Text inside a quoted Mermaid label.
fn mermaid_text(text: &str) -> String {
    text.replace('"', "#quot;")
        .replace('\n', "<br/>")
        .replace('\r', "")
}

/// The diagram as a Mermaid `flowchart`: groups as subgraphs, node kinds as
/// shapes, edge styles as arrows, statuses as classes, code refs as comments.
pub fn mermaid(d: &Diagram, title: &str) -> String {
    let nodes = mermaid_ids("n_", d.nodes.iter().map(|n| n.id.as_str()));
    let groups = mermaid_ids("g_", d.groups.iter().map(|g| g.id.as_str()));
    let node_id = |id: &str| d.node_index(id).map(|i| nodes[i].as_str());
    let mut out = String::new();
    if !title.trim().is_empty() {
        let _ = writeln!(out, "---\ntitle: {}\n---", title.replace('\n', " "));
    }
    let _ = writeln!(out, "flowchart {}", d.direction.name());
    let node_line = |out: &mut String, i: usize, indent: &str| {
        let node = &d.nodes[i];
        let mut label = mermaid_text(&node.label);
        if let Some(tech) = &node.tech {
            label.push_str(&format!("<br/><small>{}</small>", mermaid_text(tech)));
        }
        let (open, close) = match node.kind {
            NodeKind::Database | NodeKind::Storage => ("[(", ")]"),
            NodeKind::Queue => ("[[", "]]"),
            NodeKind::Cache => ("{{", "}}"),
            NodeKind::Client => ("([", "])"),
            NodeKind::External => (">", "]"),
            NodeKind::Function => ("(", ")"),
            NodeKind::Service | NodeKind::Note => ("[", "]"),
        };
        let _ = writeln!(out, "{indent}{}{open}\"{label}\"{close}", nodes[i]);
    };
    for (gi, group) in d.groups.iter().enumerate() {
        let members: Vec<usize> = (0..d.nodes.len())
            .filter(|&i| d.nodes[i].group.as_deref() == Some(group.id.as_str()))
            .collect();
        let _ = writeln!(
            out,
            "  subgraph {}[\"{}\"]",
            groups[gi],
            mermaid_text(&group.label)
        );
        for i in members {
            node_line(&mut out, i, "    ");
        }
        let _ = writeln!(out, "  end");
    }
    for i in 0..d.nodes.len() {
        if d.nodes[i].group.is_none() {
            node_line(&mut out, i, "  ");
        }
    }
    for edge in &d.edges {
        let (Some(from), Some(to)) = (node_id(&edge.from), node_id(&edge.to)) else {
            continue;
        };
        let arrow = match edge.style {
            EdgeStyle::Sync => "-->",
            EdgeStyle::Async => "-.->",
            EdgeStyle::Data => "==>",
            EdgeStyle::Dependency => "--o",
        };
        match &edge.label {
            Some(label) => {
                let _ = writeln!(out, "  {from} {arrow}|\"{}\"| {to}", mermaid_text(label));
            }
            None => {
                let _ = writeln!(out, "  {from} {arrow} {to}");
            }
        }
    }
    let classes = [
        (Status::Planned, "stroke-dasharray: 5 5"),
        (Status::Deprecated, "opacity: 0.55"),
        (Status::Risk, "stroke: #dc2626, stroke-width: 2px"),
    ];
    for (status, style) in classes {
        let ids: Vec<&str> = (0..d.nodes.len())
            .filter(|&i| d.nodes[i].status == Some(status))
            .map(|i| nodes[i].as_str())
            .collect();
        if !ids.is_empty() {
            let _ = writeln!(out, "  classDef {} {style}", status.name());
            let _ = writeln!(out, "  class {} {}", ids.join(","), status.name());
        }
    }
    for (i, node) in d.nodes.iter().enumerate() {
        if let Some(code_ref) = &node.code_ref {
            let _ = writeln!(out, "  %% {}: {}", nodes[i], code_ref.text());
        }
    }
    out
}

// ── SVG ───────────────────────────────────────────────────────────────────

fn xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if (c as u32) < 0x20 && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

/// `text` cut to `max` characters for a fixed-size box.
fn fit(text: &str, max: usize) -> String {
    super::diagram::truncate(text, max)
}

fn svg_open(out: &mut String, w: f32, h: f32, palette: &Palette) {
    let _ = writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w:.0}" height="{h:.0}" viewBox="0 0 {w:.0} {h:.0}" font-family="{}">"#,
        xml(&palette.font)
    );
    let _ = writeln!(
        out,
        r#"<rect width="100%" height="100%" fill="{}"/>"#,
        palette.background
    );
}

fn tone(palette: &Palette, i: usize) -> &str {
    palette
        .series
        .get(i % palette.series.len().max(1))
        .map_or(palette.accent.as_str(), String::as_str)
}

fn kind_tone(kind: NodeKind) -> usize {
    match kind {
        NodeKind::Service | NodeKind::Function => 0,
        NodeKind::Client => 1,
        NodeKind::Database | NodeKind::Storage => 2,
        NodeKind::Queue => 3,
        NodeKind::Cache => 4,
        NodeKind::External | NodeKind::Note => 7,
    }
}

/// The diagram as laid out in the app, as a standalone SVG.
pub fn diagram_svg(d: &Diagram, palette: &Palette) -> String {
    const MARGIN: f32 = 24.;
    let l = layout::layout(d);
    let b = l.bounds;
    let (ox, oy) = (MARGIN - b.x, MARGIN - b.y);
    let at = |p: Point| (p.x + ox, p.y + oy);
    let mut out = String::new();
    svg_open(
        &mut out,
        b.w.max(1.) + MARGIN * 2.,
        b.h.max(1.) + MARGIN * 2.,
        palette,
    );
    let _ = writeln!(
        out,
        r#"<defs><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="{}"/></marker></defs>"#,
        palette.muted
    );
    for (group, rect) in d.groups.iter().zip(&l.groups) {
        let Some(r) = rect else { continue };
        let (x, y) = at(Point { x: r.x, y: r.y });
        let _ = writeln!(
            out,
            r#"<rect x="{x:.1}" y="{y:.1}" width="{:.1}" height="{:.1}" rx="14" fill="{}" fill-opacity="0.55" stroke="{}"/>"#,
            r.w, r.h, palette.surface, palette.border
        );
        let _ = writeln!(
            out,
            r#"<text x="{:.1}" y="{:.1}" font-size="12" fill="{}">{}</text>"#,
            x + 12.,
            y + GROUP_HEADER / 2. + 6.,
            palette.muted,
            xml(&fit(&group.label, 40))
        );
    }
    for (edge, path) in d.edges.iter().zip(&l.edges) {
        let mut data = String::new();
        if let Some(first) = path.segments.first() {
            let (x, y) = at(first.from);
            let _ = write!(data, "M{x:.1},{y:.1}");
        }
        for s in &path.segments {
            let (c1, c2, to) = (at(s.c1), at(s.c2), at(s.to));
            let _ = write!(
                data,
                " C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}",
                c1.0, c1.1, c2.0, c2.1, to.0, to.1
            );
        }
        let (dash, width, color) = match edge.style {
            EdgeStyle::Sync => ("", 1.5, palette.muted.as_str()),
            EdgeStyle::Async => (r#" stroke-dasharray="6 4""#, 1.5, tone(palette, 3)),
            EdgeStyle::Data => (r#" stroke-dasharray="8 4""#, 2.5, tone(palette, 0)),
            EdgeStyle::Dependency => (r#" stroke-dasharray="2 4""#, 1.5, palette.muted.as_str()),
        };
        let _ = writeln!(
            out,
            r#"<path d="{data}" fill="none" stroke="{color}" stroke-width="{width}"{dash} marker-end="url(#arrow)"/>"#
        );
        if let Some(label) = &edge.label {
            let (x, y) = at(path.label_at());
            let text = fit(label, 30);
            let w = text.chars().count() as f32 * 6.4 + 14.;
            let _ = writeln!(
                out,
                r#"<rect x="{:.1}" y="{:.1}" width="{w:.1}" height="20" rx="10" fill="{}" stroke="{}"/><text x="{x:.1}" y="{:.1}" font-size="11" text-anchor="middle" fill="{}">{}</text>"#,
                x - w / 2.,
                y - 10.,
                palette.surface,
                palette.border,
                y + 4.,
                palette.muted,
                xml(&text)
            );
        }
    }
    for (node, r) in d.nodes.iter().zip(&l.nodes) {
        let (x, y) = at(Point { x: r.x, y: r.y });
        let dash = if node.status == Some(Status::Planned) {
            r#" stroke-dasharray="5 4""#
        } else {
            ""
        };
        let stroke = if node.status == Some(Status::Risk) {
            tone(palette, 6)
        } else {
            palette.border.as_str()
        };
        let opacity = if node.status == Some(Status::Deprecated) {
            r#" opacity="0.55""#
        } else {
            ""
        };
        let _ = writeln!(
            out,
            r#"<g{opacity}><rect x="{x:.1}" y="{y:.1}" width="{:.1}" height="{:.1}" rx="10" fill="{}" stroke="{stroke}"{dash}/><rect x="{:.1}" y="{:.1}" width="6" height="{:.1}" rx="3" fill="{}"/>"#,
            r.w,
            r.h,
            palette.surface,
            x + 8.,
            y + 10.,
            r.h - 20.,
            tone(palette, kind_tone(node.kind)),
        );
        let sub = node.tech.as_deref().unwrap_or(node.kind.name());
        let _ = writeln!(
            out,
            r#"<text x="{:.1}" y="{:.1}" font-size="13" font-weight="600" fill="{}">{}</text><text x="{:.1}" y="{:.1}" font-size="11" fill="{}">{}</text></g>"#,
            x + 22.,
            y + r.h / 2. - 3.,
            palette.text,
            xml(&fit(&node.label, 22)),
            x + 22.,
            y + r.h / 2. + 13.,
            palette.muted,
            xml(&fit(sub, 26)),
        );
    }
    out.push_str("</svg>\n");
    out
}

// ── Tables ────────────────────────────────────────────────────────────────

/// One CSV field (RFC 4180): quoted when it holds a comma, a quote, a line
/// break or edge spaces; quotes doubled.
pub fn csv_field(text: &str) -> String {
    let needs =
        text.contains([',', '"', '\n', '\r']) || text.starts_with(' ') || text.ends_with(' ');
    if needs {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

fn csv_line(out: &mut String, fields: impl IntoIterator<Item = String>) {
    let fields: Vec<String> = fields.into_iter().map(|f| csv_field(&f)).collect();
    out.push_str(&fields.join(","));
    out.push('\n');
}

fn has_refs(table: &Table) -> bool {
    table.rows.iter().any(|r| r.code_ref.is_some())
}

/// The table as CSV: a header of column labels, one line per row (links as
/// their URL), and a trailing `ref` column when rows point at code.
pub fn table_csv(table: &Table) -> String {
    let refs = has_refs(table);
    let mut out = String::new();
    csv_line(
        &mut out,
        table
            .columns
            .iter()
            .map(|c| c.label.clone())
            .chain(refs.then(|| "ref".to_string())),
    );
    for row in &table.rows {
        let cells = table.columns.iter().map(|c| {
            let value = row.cells.get(&c.key).unwrap_or(&Value::Null);
            match c.kind {
                ColumnType::Link => link_url(value).map_or_else(|| cell_text(value), String::from),
                _ => cell_text(value),
            }
        });
        let code = refs.then(|| {
            row.code_ref
                .as_ref()
                .map_or_else(String::new, CodeRef::text)
        });
        csv_line(&mut out, cells.chain(code));
    }
    out
}

/// Text inside a Markdown table cell.
fn md_cell(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace("\r\n", "<br>")
        .replace('\n', "<br>")
}

/// Inline Markdown text (outside tables): line breaks become spaces.
fn md_inline(text: &str) -> String {
    text.replace("\r\n", " ").replace('\n', " ")
}

fn md_ref(code_ref: &CodeRef) -> String {
    format!("`{}`", code_ref.text().replace('`', "'"))
}

/// The table as a GitHub-flavored Markdown table, aligned like its columns.
pub fn table_markdown(table: &Table) -> String {
    let refs = has_refs(table);
    let mut out = String::new();
    let mut header: Vec<String> = table.columns.iter().map(|c| md_cell(&c.label)).collect();
    let mut rule: Vec<&str> = table
        .columns
        .iter()
        .map(|c| match c.align {
            Align::Left => "---",
            Align::Center => ":---:",
            Align::Right => "---:",
        })
        .collect();
    if refs {
        header.push("Code".into());
        rule.push("---");
    }
    let _ = writeln!(out, "| {} |", header.join(" | "));
    let _ = writeln!(out, "| {} |", rule.join(" | "));
    for row in &table.rows {
        let mut cells: Vec<String> = table
            .columns
            .iter()
            .map(|c| {
                let value = row.cells.get(&c.key).unwrap_or(&Value::Null);
                let text = md_cell(&cell_text(value));
                match (c.kind, link_url(value)) {
                    (ColumnType::Link, Some(url)) => {
                        format!("[{text}]({})", url.replace(' ', "%20"))
                    }
                    _ => text,
                }
            })
            .collect();
        if refs {
            cells.push(row.code_ref.as_ref().map_or_else(String::new, md_ref));
        }
        let _ = writeln!(out, "| {} |", cells.join(" | "));
    }
    out
}

// ── Checklists and boards ─────────────────────────────────────────────────

/// The checklist as a Markdown task list (sub-items indented).
pub fn checklist_markdown(list: &Checklist) -> String {
    let mut out = String::new();
    for (parent, item) in list.flat() {
        let indent = if parent.is_some() { "  " } else { "" };
        let _ = write!(
            out,
            "{indent}- [{}] {}",
            if item.done { "x" } else { " " },
            md_inline(&item.label)
        );
        if let Some(code_ref) = &item.code_ref {
            let _ = write!(out, " ({})", md_ref(code_ref));
        }
        if let Some(note) = &item.note {
            let _ = write!(out, " — {}", md_inline(note));
        }
        out.push('\n');
    }
    out
}

/// The board as Markdown: a heading per column and its cards as a list.
pub fn kanban_markdown(board: &Board) -> String {
    let mut out = String::new();
    for (i, column) in board.columns.iter().enumerate() {
        let cards: Vec<_> = board
            .cards
            .iter()
            .filter(|c| c.column == column.id)
            .collect();
        if i > 0 {
            out.push('\n');
        }
        let count = match column.limit {
            Some(limit) => format!("{}/{limit}", cards.len()),
            None => cards.len().to_string(),
        };
        let _ = writeln!(out, "### {} ({count})\n", md_inline(&column.label));
        if cards.is_empty() {
            out.push_str("_No cards_\n");
        }
        for card in cards {
            let _ = write!(out, "- **{}**", md_inline(&card.title));
            if let Some(description) = &card.description {
                let _ = write!(out, " — {}", md_inline(description));
            }
            for tag in &card.tags {
                let _ = write!(out, " `{}`", tag.replace('`', "'"));
            }
            if let Some(assignee) = &card.assignee {
                let _ = write!(out, " @{}", md_inline(assignee));
            }
            if let Some(code_ref) = &card.code_ref {
                let _ = write!(out, " ({})", md_ref(code_ref));
            }
            out.push('\n');
        }
    }
    out
}

// ── Charts ────────────────────────────────────────────────────────────────

fn number_text(n: f64) -> String {
    super::edits::format_number(n)
}

/// The chart's data as CSV: `label,<series…>` per label, or `series,x,y`
/// per point for scatter.
pub fn chart_csv(chart: &Chart) -> String {
    let mut out = String::new();
    if chart.kind == ChartType::Scatter {
        csv_line(&mut out, ["series", "x", "y"].map(String::from));
        for s in &chart.series {
            for (x, y) in &s.points {
                csv_line(&mut out, [s.name.clone(), number_text(*x), number_text(*y)]);
            }
        }
        return out;
    }
    let series: Vec<_> = if chart.kind.is_round() {
        chart.series.iter().take(1).collect()
    } else {
        chart.series.iter().collect()
    };
    csv_line(
        &mut out,
        std::iter::once(chart.x_title.clone().unwrap_or_else(|| "label".into()))
            .chain(series.iter().map(|s| s.name.clone())),
    );
    for (i, label) in chart.labels.iter().enumerate() {
        csv_line(
            &mut out,
            std::iter::once(label.clone()).chain(series.iter().map(|s| {
                s.values
                    .get(i)
                    .map_or_else(String::new, |v| number_text(*v))
            })),
        );
    }
    out
}

/// The chart drawn like the app draws it, as a standalone SVG (720 × 440)
/// with its title, axes and a legend.
pub fn chart_svg(chart: &Chart, title: &str, palette: &Palette) -> String {
    const W: f32 = 720.;
    const H: f32 = 440.;
    let mut out = String::new();
    svg_open(&mut out, W, H, palette);
    let _ = writeln!(
        out,
        r#"<text x="24" y="32" font-size="16" font-weight="600" fill="{}">{}</text>"#,
        palette.text,
        xml(&fit(title, 80))
    );
    let legend = chart.kind.is_round() || chart.series.len() > 1;
    let bottom = if legend { 74. } else { 48. };
    let axes = Axes::of(chart);
    let plot = if chart.kind.is_round() {
        Rect {
            x: 24.,
            y: 52.,
            w: W - 48.,
            h: H - 52. - bottom,
        }
    } else {
        Rect {
            x: 72.,
            y: 52.,
            w: W - 96.,
            h: H - 52. - bottom,
        }
    };
    let g = Geometry::new(chart, axes.as_ref(), plot, 1.);
    if let Some(axes) = &axes {
        for tick in &axes.y.ticks {
            let y = plot.y + plot.h * (1. - axes.y.fraction(*tick));
            let _ = writeln!(
                out,
                r#"<line x1="{:.1}" y1="{y:.1}" x2="{:.1}" y2="{y:.1}" stroke="{}"/><text x="{:.1}" y="{:.1}" font-size="11" text-anchor="end" fill="{}">{}</text>"#,
                plot.x,
                plot.right(),
                palette.border,
                plot.x - 8.,
                y + 4.,
                palette.muted,
                xml(&short_number(*tick))
            );
        }
        match &axes.x {
            Some(x_axis) => {
                for tick in &x_axis.ticks {
                    let x = plot.x + plot.w * x_axis.fraction(*tick);
                    let _ = writeln!(
                        out,
                        r#"<text x="{x:.1}" y="{:.1}" font-size="11" text-anchor="middle" fill="{}">{}</text>"#,
                        plot.bottom() + 18.,
                        palette.muted,
                        xml(&short_number(*tick))
                    );
                }
            }
            None => {
                for (label, x) in chart.labels.iter().zip(&g.columns) {
                    let _ = writeln!(
                        out,
                        r#"<text x="{x:.1}" y="{:.1}" font-size="11" text-anchor="middle" fill="{}">{}</text>"#,
                        plot.bottom() + 18.,
                        palette.muted,
                        xml(&fit(label, 16))
                    );
                }
            }
        }
        if let Some(y_title) = &chart.y_title {
            let _ = writeln!(
                out,
                r#"<text x="16" y="{:.1}" font-size="11" fill="{}" transform="rotate(-90 16 {:.1})" text-anchor="middle">{}</text>"#,
                plot.y + plot.h / 2.,
                palette.muted,
                plot.y + plot.h / 2.,
                xml(y_title)
            );
        }
        if let Some(x_title) = &chart.x_title {
            let _ = writeln!(
                out,
                r#"<text x="{:.1}" y="{:.1}" font-size="11" text-anchor="middle" fill="{}">{}</text>"#,
                plot.x + plot.w / 2.,
                plot.bottom() + 36.,
                palette.muted,
                xml(x_title)
            );
        }
    }
    let points = |line: &[Point]| {
        line.iter()
            .map(|p| format!("{:.1},{:.1}", p.x, p.y))
            .collect::<Vec<_>>()
            .join(" ")
    };
    if chart.kind == ChartType::Area {
        for (si, (line, floor)) in g.lines.iter().zip(&g.floors).enumerate() {
            let mut shape = points(line);
            let back: Vec<Point> = floor.iter().rev().copied().collect();
            shape.push(' ');
            shape.push_str(&points(&back));
            let _ = writeln!(
                out,
                r#"<polygon points="{shape}" fill="{}" fill-opacity="0.25"/>"#,
                tone(palette, si)
            );
        }
    }
    if matches!(chart.kind, ChartType::Line | ChartType::Area) {
        for (si, line) in g.lines.iter().enumerate() {
            let _ = writeln!(
                out,
                r#"<polyline points="{}" fill="none" stroke="{}" stroke-width="2"/>"#,
                points(line),
                tone(palette, si)
            );
        }
    }
    for mark in &g.marks {
        let color = if chart.kind.is_round() {
            tone(palette, mark.index)
        } else {
            tone(palette, mark.series)
        };
        match &mark.shape {
            Shape::Bar(r) => {
                let _ = writeln!(
                    out,
                    r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="2" fill="{color}"/>"#,
                    r.x, r.y, r.w, r.h
                );
            }
            Shape::Dot(p) => {
                let _ = writeln!(
                    out,
                    r#"<circle cx="{:.1}" cy="{:.1}" r="3.5" fill="{color}"/>"#,
                    p.x, p.y
                );
            }
            Shape::Slice {
                center,
                radius,
                inner,
                start,
                end,
            } => {
                let _ = writeln!(
                    out,
                    r#"<path d="{}" fill="{color}" stroke="{}" stroke-width="1.5"/>"#,
                    slice_path(*center, *radius, *inner, *start, *end),
                    palette.background
                );
            }
        }
    }
    if legend {
        let names: Vec<String> = if chart.kind.is_round() {
            chart.labels.clone()
        } else {
            chart.series.iter().map(|s| s.name.clone()).collect()
        };
        let mut x = 24.;
        let y = H - 24.;
        for (i, name) in names.iter().enumerate() {
            let text = fit(name, 24);
            let w = text.chars().count() as f32 * 6.6 + 30.;
            if x + w > W - 24. {
                break;
            }
            let _ = writeln!(
                out,
                r#"<rect x="{x:.1}" y="{:.1}" width="10" height="10" rx="2" fill="{}"/><text x="{:.1}" y="{y:.1}" font-size="11" fill="{}">{}</text>"#,
                y - 9.,
                tone(palette, i),
                x + 15.,
                palette.muted,
                xml(&text)
            );
            x += w;
        }
    }
    out.push_str("</svg>\n");
    out
}

/// A pie or donut slice, angles clockwise from 12 o'clock.
fn slice_path(c: Point, r: f32, inner: f32, start: f32, end: f32) -> String {
    let at = |radius: f32, angle: f32| (c.x + radius * angle.sin(), c.y - radius * angle.cos());
    // A full circle as two halves (an arc can't start where it ends).
    let sweep = (end - start).min(std::f32::consts::TAU - 1e-3);
    let end = start + sweep;
    let large = i32::from(sweep > std::f32::consts::PI);
    let (sx, sy) = at(r, start);
    let (ex, ey) = at(r, end);
    if inner <= 0. {
        return format!(
            "M{:.1},{:.1} L{sx:.1},{sy:.1} A{r:.1},{r:.1} 0 {large} 1 {ex:.1},{ey:.1} Z",
            c.x, c.y
        );
    }
    let (ix, iy) = at(inner, end);
    let (jx, jy) = at(inner, start);
    format!(
        "M{sx:.1},{sy:.1} A{r:.1},{r:.1} 0 {large} 1 {ex:.1},{ey:.1} L{ix:.1},{iy:.1} A{inner:.1},{inner:.1} 0 {large} 0 {jx:.1},{jy:.1} Z"
    )
}

// ── Forms ─────────────────────────────────────────────────────────────────

/// The form's values (answers, else defaults) by field id, in field order.
pub fn form_json(form: &Form) -> String {
    let values: Map<String, Value> = form
        .fields
        .iter()
        .map(|f| (f.id.clone(), form.value(&f.id)))
        .collect();
    let mut text = serde_json::to_string_pretty(&Value::Object(values)).unwrap_or_default();
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn diagram() -> Diagram {
        Diagram::parse(&json!({
            "direction": "TB",
            "groups": [{"id": "backend", "label": "Back \"end\""}],
            "nodes": [
                {"id": "web", "label": "Web app", "kind": "client"},
                {"id": "api", "label": "API", "group": "backend", "tech": "Rust", "status": "planned", "ref": "src/api.rs:12"},
                {"id": "end", "label": "Orders DB", "kind": "database", "group": "backend"},
                {"id": "q-1", "label": "Jobs", "kind": "queue"},
                {"id": "q_1", "label": "Jobs 2", "kind": "queue"}
            ],
            "edges": [
                {"from": "web", "to": "api", "label": "HTTPS | json"},
                {"from": "api", "to": "end", "style": "data"},
                {"from": "api", "to": "q-1", "style": "async"},
                {"from": "q-1", "to": "q_1", "style": "dependency"}
            ]
        }))
        .unwrap()
    }

    /// Mermaid read back structurally: the header, balanced subgraphs,
    /// every node declared once and every edge between declared nodes.
    #[test]
    fn mermaid_is_a_well_formed_flowchart() {
        let text = mermaid(&diagram(), "Checkout");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            &lines[..4],
            ["---", "title: Checkout", "---", "flowchart TB"]
        );
        let mut depth = 0;
        let mut declared: Vec<String> = Vec::new();
        let mut edges = 0;
        for line in &lines[4..] {
            let line = line.trim();
            if line.starts_with("subgraph ") {
                depth += 1;
                assert!(line.contains("[\"Back #quot;end#quot;\"]"), "{line}");
                continue;
            }
            if line == "end" {
                depth -= 1;
                assert!(depth >= 0);
                continue;
            }
            if line.starts_with("classDef") || line.starts_with("class ") || line.starts_with("%%")
            {
                continue;
            }
            let id: String = line
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            assert!(id.starts_with("n_"), "{line}");
            let rest = &line[id.len()..];
            if let Some(arrow) = ["-->", "-.->", "==>", "--o"]
                .iter()
                .find(|a| rest.trim_start().starts_with(**a))
            {
                let target = rest
                    .trim_start()
                    .trim_start_matches(arrow)
                    .rsplit(' ')
                    .next()
                    .unwrap();
                assert!(declared.contains(&id), "{id} used before it is declared");
                assert!(
                    declared.iter().any(|d| d == target),
                    "unknown target {target}"
                );
                edges += 1;
            } else {
                assert!(!declared.contains(&id), "{id} declared twice");
                declared.push(id);
            }
        }
        assert_eq!(depth, 0);
        assert_eq!(declared.len(), 5);
        assert_eq!(edges, 4);
        // `end` is a keyword: ids are prefixed; `q-1` and `q_1` stay apart.
        assert!(text.contains("n_end[(\"Orders DB\")]"));
        assert!(text.contains("n_q_1[[\"Jobs\"]]") && text.contains("n_q_1_2[[\"Jobs 2\"]]"));
        assert!(text.contains("n_web -->|\"HTTPS | json\"| n_api"));
        assert!(text.contains("n_api ==> n_end") && text.contains("n_api -.-> n_q_1"));
        assert!(text.contains("n_api[\"API<br/><small>Rust</small>\"]"));
        assert!(text.contains("class n_api planned"));
        assert!(text.contains("%% n_api: src/api.rs:12"));
    }

    #[test]
    fn diagram_svg_draws_every_node_and_edge() {
        let svg = diagram_svg(&diagram(), &Palette::default());
        assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(svg.trim_end().ends_with("</svg>"));
        assert_eq!(svg.matches("marker-end=").count(), 4);
        assert!(svg.contains(">Orders DB</text>"));
        assert!(svg.contains("Back &quot;end&quot;"));
        assert!(svg.contains("HTTPS | json"));
    }

    fn table() -> Table {
        Table::parse(&json!({
            "columns": [
                {"key": "name", "label": "Name"},
                {"key": "n", "label": "Count", "type": "number"},
                {"key": "url", "label": "Link", "type": "link"}
            ],
            "rows": [
                {"id": "a", "cells": {"name": "plain", "n": 3, "url": {"text": "docs", "url": "https://x.dev/a b"}}, "ref": "src/a.rs:4"},
                {"id": "b", "cells": {"name": "with, comma \"quoted\"\nand a | pipe", "n": 1.5}}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn csv_escapes_fields() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("two\nlines"), "\"two\nlines\"");
        assert_eq!(csv_field(" padded"), "\" padded\"");
        assert_eq!(
            table_csv(&table()),
            "Name,Count,Link,ref\n\
             plain,3,https://x.dev/a b,src/a.rs:4\n\
             \"with, comma \"\"quoted\"\"\nand a | pipe\",1.5,,\n"
        );
    }

    #[test]
    fn markdown_tables_align_and_escape() {
        assert_eq!(
            table_markdown(&table()),
            "| Name | Count | Link | Code |\n\
             | --- | ---: | --- | --- |\n\
             | plain | 3 | [docs](https://x.dev/a%20b) | `src/a.rs:4` |\n\
             | with, comma \"quoted\"<br>and a \\| pipe | 1.5 |  |  |\n"
        );
    }

    #[test]
    fn checklists_and_boards_as_markdown() {
        let list = Checklist::parse(&json!({"items": [
            {"id": "plan", "label": "Plan", "done": true, "note": "with\nnotes"},
            {"id": "build", "label": "Build", "items": [{"id": "api", "label": "API", "ref": "src/api.rs"}]}
        ]}))
        .unwrap();
        assert_eq!(
            checklist_markdown(&list),
            "- [x] Plan — with notes\n- [ ] Build\n  - [ ] API (`src/api.rs`)\n"
        );
        let board = Board::parse(&json!({
            "columns": [{"id": "todo", "label": "To do", "limit": 2}, "Done"],
            "cards": [{"id": "c", "title": "OAuth", "description": "login", "tags": ["auth"], "assignee": "Ada", "ref": "src/auth.rs:9"}]
        }))
        .unwrap();
        assert_eq!(
            kanban_markdown(&board),
            "### To do (1/2)\n\n- **OAuth** — login `auth` @Ada (`src/auth.rs:9`)\n\n### Done (0)\n\n_No cards_\n"
        );
    }

    #[test]
    fn chart_data_and_picture() {
        let chart = Chart::parse(&json!({
            "type": "bar", "labels": ["Mon", "Tue"],
            "series": [{"name": "api", "values": [120, 98.5]}, {"name": "web, edge", "values": [3, 4]}]
        }))
        .unwrap();
        assert_eq!(
            chart_csv(&chart),
            "label,api,\"web, edge\"\nMon,120,3\nTue,98.5,4\n"
        );
        let svg = chart_svg(&chart, "p95 <latency>", &Palette::default());
        assert_eq!(
            svg.matches("<rect x=").count() - 2,
            4,
            "4 bars and 2 legend keys"
        );
        assert!(svg.contains("p95 &lt;latency&gt;"));
        let scatter = Chart::parse(
            &json!({"type": "scatter", "series": [{"name": "s", "points": [[1, 2], [3.5, 4]]}]}),
        )
        .unwrap();
        assert_eq!(chart_csv(&scatter), "series,x,y\ns,1,2\ns,3.5,4\n");
        let pie = Chart::parse(&json!({"type": "donut", "labels": ["a", "b"], "series": [{"name": "s", "values": [1, 3]}]})).unwrap();
        assert_eq!(
            chart_svg(&pie, "", &Palette::default())
                .matches("<path d=\"M")
                .count(),
            2
        );
    }

    #[test]
    fn form_values_as_json() {
        let form = Form::parse(&json!({
            "fields": [
                {"id": "env", "type": "radio", "label": "Env", "options": ["staging", "prod"]},
                {"id": "notes", "type": "textarea", "label": "Notes"}
            ],
            "values": {"env": "prod"}
        }))
        .unwrap();
        let value: Value = serde_json::from_str(&form_json(&form)).unwrap();
        assert_eq!(value["env"], "prod");
        assert!(value.get("notes").is_some());
    }

    #[test]
    fn render_checks_the_format_and_names_files() {
        let spec = json!({"html": "<p>hi</p>"});
        assert_eq!(
            render("html", "x", &spec, Format::Html, &Palette::default()).unwrap(),
            "<p>hi</p>"
        );
        assert!(render("html", "x", &spec, Format::Csv, &Palette::default()).is_err());
        assert!(render("table", "x", &json!({}), Format::Csv, &Palette::default()).is_err());
        assert_eq!(
            file_name("Checkout — v2!", Format::Mermaid),
            "checkout-v2.mmd"
        );
        assert_eq!(file_name("", Format::Png), "ui.png");
        assert_eq!(docs_format("chart"), Some(Format::Csv));
        assert_eq!(docs_format("diagram"), Some(Format::Mermaid));
    }

    #[test]
    fn docs_command_carries_the_docs_format() {
        let spec = json!({"nodes": [{"id": "api"}]});
        let command = docs_command("arch", "diagram", "Arch", &spec).unwrap();
        let event: Value =
            serde_json::from_str(command.strip_prefix("/gentle-ui:event ").unwrap()).unwrap();
        assert_eq!(event["op"], "docs");
        assert_eq!(event["id"], "arch");
        assert!(event.get("version").is_none());
        assert_eq!(
            (event["format"].as_str(), event["fence"].as_str()),
            (Some("Mermaid"), Some("mermaid"))
        );
        assert!(event["text"].as_str().unwrap().contains("flowchart LR"));
        let big = json!({"html": "x".repeat(MAX_DOCS_TEXT + 1)});
        let event: Value = serde_json::from_str(
            docs_command("p", "html", "Page", &big)
                .unwrap()
                .strip_prefix("/gentle-ui:event ")
                .unwrap(),
        )
        .unwrap();
        assert!(event["text"].is_null());
    }
}
