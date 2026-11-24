//! Keyboard shortcuts as data: what can follow a pending key sequence (the
//! which-key hint) and the keymap grouped for the cheat-sheet screen.
//!
//! The app reads its live keymap into [`Binding`]s (keystrokes as GPUI
//! binding strings such as `"ctrl-space"`, the action's name and its key
//! context) and this module decides what to show. Only actions with a
//! human label in [`label`] are listed; editor plumbing stays out.

use crate::keyboard::format_binding;

/// One key binding of the live keymap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// The keystrokes, one binding string each (`["ctrl-space", "h"]`).
    pub keys: Vec<String>,
    /// The action's registered name (`"gentle::NewTab"`).
    pub action: String,
    /// The key context predicate it is bound in, if any.
    pub context: Option<String>,
}

/// A key that can continue the pending sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    /// What to press next, as binding strings. More than one when several
    /// bindings reach the same action; more than one string per sequence
    /// when the binding is still longer than one more key (`"g g"`).
    pub keys: Vec<String>,
    pub label: &'static str,
}

/// The cheat sheet's groups, in the order they are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Direct,
    Leader,
    PaneMode,
    Files,
    Session,
}

impl Section {
    pub const ALL: [Section; 5] = [
        Section::Direct,
        Section::Leader,
        Section::PaneMode,
        Section::Files,
        Section::Session,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Section::Direct => "Direct",
            Section::Leader => "Leader",
            Section::PaneMode => "Pane mode",
            Section::Files => "Files & Git changes",
            Section::Session => "Composer & session",
        }
    }
}

/// One line of the cheat sheet: an action and every way to press it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub label: &'static str,
    /// Binding strings, each a whole sequence (`"ctrl-space h"`).
    pub keys: Vec<String>,
    /// Only works while the "Vim keys" setting is on.
    pub vim: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub section: Section,
    pub rows: Vec<Row>,
}

/// The human label of an action, or `None` for actions that are not
/// shortcuts worth listing.
pub fn label(action: &str) -> Option<&'static str> {
    Some(match action {
        "gentle::NewTab" => "New agent tab",
        "gentle::NewTerminalTab" => "New terminal tab",
        "gentle::NewCanvasTab" => "New canvas tab",
        "gentle::CloseTab" => "Close tab",
        "gentle::NextTab" => "Next tab",
        "gentle::PrevTab" => "Previous tab",
        "gentle::SplitRight" => "Split right",
        "gentle::SplitDown" => "Split down",
        "gentle::SplitLeft" => "Split left",
        "gentle::SplitUp" => "Split up",
        "gentle::TilePane" => "New pane (tiled)",
        "gentle::ClosePane" => "Close pane",
        "gentle::ReopenClosed" => "Reopen closed pane",
        "gentle::FocusNextPane" => "Focus next pane",
        "gentle::FocusPrevPane" => "Focus previous pane",
        "gentle::FocusPaneLeft" => "Focus pane left",
        "gentle::FocusPaneRight" => "Focus pane right",
        "gentle::FocusPaneUp" => "Focus pane up",
        "gentle::FocusPaneDown" => "Focus pane down",
        "gentle::ResizePaneLeft" => "Resize pane left",
        "gentle::ResizePaneRight" => "Resize pane right",
        "gentle::ResizePaneUp" => "Resize pane up",
        "gentle::ResizePaneDown" => "Resize pane down",
        "gentle::TogglePaneMode" => "Pane mode",
        "gentle::ExitPaneMode" => "Exit pane mode",
        "gentle::ToggleSidebar" => "Toggle Files sidebar",
        "gentle::FocusComposer" => "Focus composer",
        "gentle::FocusFiles" => "Focus Files",
        "gentle::FocusChanges" => "Focus Git changes",
        "gentle::OpenPalette" => "Command palette",
        "gentle::OpenShortcuts" => "Keyboard shortcuts",
        "gentle::OpenSessions" => "Sessions",
        "gentle::OpenWorkspaces" => "Switch workspace…",
        "gentle::GoToTab" => "Go to tab…",
        "gentle::FocusTabList" => "Browse tabs",
        "gentle::CreateWorkspace" => "Create workspace",
        "gentle::DeleteWorkspace" => "Delete workspace…",
        "gentle::OpenSettings" => "Settings",
        "gentle::CloseOverlay" => "Close screen",
        "gentle::Quit" => "Quit",
        "gentle::ZoomIn" => "Zoom in",
        "gentle::ZoomOut" => "Zoom out",
        "gentle::ActualSize" => "Actual size",
        "gentle::ActivateTab1" => "Go to tab 1",
        "gentle::ActivateTab2" => "Go to tab 2",
        "gentle::ActivateTab3" => "Go to tab 3",
        "gentle::ActivateTab4" => "Go to tab 4",
        "gentle::ActivateTab5" => "Go to tab 5",
        "gentle::ActivateTab6" => "Go to tab 6",
        "gentle::ActivateTab7" => "Go to tab 7",
        "gentle::ActivateTab8" => "Go to tab 8",
        "gentle::ActivateTab9" => "Go to last tab (9)",
        "session::Abort" => "Interrupt the agent",
        "session::Stop" => "Stop the agent",
        "session::ToggleAgents" => "Agents panel",
        "session::NewSession" => "New session",
        "file_tree::Open" => "Files · open",
        "file_tree::OpenToSide" => "Files · open to the side",
        "file_tree::OpenInNewTab" => "Files · open in a new tab",
        "file_tree::InsertPath" => "Files · insert path in the composer",
        "file_tree::CopyPath" => "Files · copy path",
        "file_tree::CopyRelativePath" => "Files · copy relative path",
        "file_tree::RevealInFinder" => "Files · reveal in the file manager",
        "file_tree::Rename" => "Files · rename",
        "file_tree::CutEntry" => "Files · cut",
        "file_tree::CopyEntry" => "Files · copy",
        "file_tree::PasteEntry" => "Files · paste",
        "file_tree::MoveToTrash" => "Files · move to trash",
        "file_tree::SelectPrev" => "Files · previous",
        "file_tree::SelectNext" => "Files · next",
        "file_tree::SelectFirst" => "Files · first",
        "file_tree::SelectLast" => "Files · last",
        "file_tree::CollapseSelected" => "Files · collapse",
        "file_tree::ExpandSelected" => "Files · expand",
        "changes::SelectPrev" => "Changes · previous",
        "changes::SelectNext" => "Changes · next",
        "changes::SelectFirst" => "Changes · first",
        "changes::SelectLast" => "Changes · last",
        "changes::OpenSelected" => "Changes · open diff",
        "changes::ToggleSelected" => "Changes · select",
        "changes::StageSelected" => "Changes · stage",
        "changes::UnstageSelected" => "Changes · unstage",
        _ => return None,
    })
}

/// The last keystroke of a binding string (a single keystroke or the last
/// one of a sequence).
fn last_keystroke(sequence: &str) -> &str {
    sequence.split_whitespace().last().unwrap_or(sequence)
}

/// When both the glyph (`?`, `:`) and its `shift-` spelling bind the same
/// action, keep only the glyph. The physical key is the same on most
/// layouts, and the guide should show one entry.
fn collapse_shift_aliases(keys: &mut Vec<String>) {
    let has_question = keys.iter().any(|k| last_keystroke(k) == "?");
    let has_colon = keys.iter().any(|k| last_keystroke(k) == ":");
    keys.retain(|k| {
        let last = last_keystroke(k);
        !(has_question && last == "shift-/" || has_colon && last == "shift-;")
    });
}

/// The keys that can follow `pending` (binding strings), for bindings with
/// a label. Several bindings to the same action merge into one hint; the
/// keymap's order is kept. `?` and `:` are preferred over `shift-/` and
/// `shift-;`.
pub fn hints(bindings: &[Binding], pending: &[String]) -> Vec<Hint> {
    let mut hints: Vec<Hint> = Vec::new();
    for binding in bindings {
        if binding.keys.len() <= pending.len() || !binding.keys.starts_with(pending) {
            continue;
        }
        let Some(label) = label(&binding.action) else {
            continue;
        };
        let rest = binding.keys[pending.len()..].join(" ");
        match hints.iter_mut().find(|h| h.label == label) {
            Some(hint) => {
                if !hint.keys.contains(&rest) {
                    hint.keys.push(rest);
                }
            }
            None => hints.push(Hint {
                keys: vec![rest],
                label,
            }),
        }
    }
    for hint in &mut hints {
        collapse_shift_aliases(&mut hint.keys);
    }
    hints
}

/// A keystroke in one canonical spelling, so `shift-ctrl-a`, `ctrl-shift-a`
/// and (off macOS) `cmd-a`/`super-a` compare equal: modifiers in the order
/// ctrl, super, alt, shift, fn; `cmd`, `super` and `win` are the same key.
pub fn canonical_keystroke(stroke: &str) -> String {
    let mut mods = [false; 5];
    let mut rest = stroke;
    while let Some((head, tail)) = rest.split_once('-') {
        let slot = match head {
            "ctrl" | "control" => 0,
            "cmd" | "super" | "win" => 1,
            "alt" | "option" => 2,
            "shift" => 3,
            "fn" => 4,
            _ => break,
        };
        mods[slot] = true;
        rest = tail;
    }
    let mut out = String::new();
    for (on, name) in mods.iter().zip(["ctrl", "super", "alt", "shift", "fn"]) {
        if *on {
            out.push_str(name);
            out.push('-');
        }
    }
    out.push_str(rest);
    out
}

/// Whether two binding strings (a keystroke or a sequence) press the same keys.
pub fn same_keys(a: &str, b: &str) -> bool {
    let mut a = a.split_whitespace();
    let mut b = b.split_whitespace();
    loop {
        match (a.next(), b.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) if canonical_keystroke(x) == canonical_keystroke(y) => {}
            _ => return false,
        }
    }
}

fn section_of(binding: &Binding, leader: &str) -> Section {
    if binding.keys.len() > 1 && same_keys(&binding.keys[0], leader) {
        return Section::Leader;
    }
    let context = binding.context.as_deref().unwrap_or("");
    if context.contains("pane_mode") && !context.contains("!pane_mode") {
        Section::PaneMode
    } else if context.contains("FileTree") || context.contains("GentleChanges") {
        Section::Files
    } else if context.contains("GentleSession") {
        Section::Session
    } else {
        Section::Direct
    }
}

/// Group the labelled bindings into the cheat sheet's sections (empty ones
/// left out), rows alphabetical. `leader` is the leader's first keystroke.
pub fn groups(bindings: &[Binding], leader: &str) -> Vec<Group> {
    Section::ALL
        .into_iter()
        .filter_map(|section| {
            let mut rows: Vec<Row> = Vec::new();
            for binding in bindings {
                let Some(label) = label(&binding.action) else {
                    continue;
                };
                if section_of(binding, leader) != section {
                    continue;
                }
                let keys = binding.keys.join(" ");
                let vim = binding
                    .context
                    .as_deref()
                    .is_some_and(|c| c.contains("vim"));
                match rows.iter_mut().find(|r| r.label == label) {
                    Some(row) => {
                        if !row.keys.contains(&keys) {
                            row.keys.push(keys);
                        }
                    }
                    None => rows.push(Row {
                        label,
                        keys: vec![keys],
                        vim,
                    }),
                }
            }
            for row in &mut rows {
                collapse_shift_aliases(&mut row.keys);
            }
            rows.sort_by_key(|r| r.label.to_lowercase());
            (!rows.is_empty()).then_some(Group { section, rows })
        })
        .collect()
}

/// If `selected` ends with `shift-/` (or `shift-;`) and the same action is
/// also bound to `?` (or `:`), return `selected` with the glyph spelling.
/// This keeps both bindings working while the hint shows the preferred one.
pub fn preferred_display_sequence(
    action: &str,
    selected: &str,
    bindings: &[Binding],
) -> Option<String> {
    let last = last_keystroke(selected);
    let preferred = match last {
        "shift-/"
            if bindings
                .iter()
                .any(|b| b.action == action && last_keystroke(&b.keys.join(" ")) == "?") =>
        {
            "?"
        }
        "shift-;"
            if bindings
                .iter()
                .any(|b| b.action == action && last_keystroke(&b.keys.join(" ")) == ":") =>
        {
            ":"
        }
        _ => return None,
    };
    Some(
        selected
            .split_whitespace()
            .map(|part| if part == last { preferred } else { part })
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// The groups that match `query`: every word has to appear in the row's
/// label, its keys as displayed or its section's title. Case-insensitive;
/// an empty query keeps everything.
pub fn search(groups: &[Group], query: &str, macos: bool) -> Vec<Group> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    groups
        .iter()
        .filter_map(|group| {
            let rows: Vec<Row> = group
                .rows
                .iter()
                .filter(|row| {
                    let keys: Vec<String> =
                        row.keys.iter().map(|k| format_binding(k, macos)).collect();
                    let haystack =
                        format!("{} {} {}", row.label, keys.join(" "), group.section.title())
                            .to_lowercase();
                    words.iter().all(|w| haystack.contains(w.as_str()))
                })
                .cloned()
                .collect();
            (!rows.is_empty()).then_some(Group {
                section: group.section,
                rows,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(keys: &[&str], action: &str, context: Option<&str>) -> Binding {
        Binding {
            keys: keys.iter().map(|k| k.to_string()).collect(),
            action: action.into(),
            context: context.map(str::to_string),
        }
    }

    fn pending(keys: &[&str]) -> Vec<String> {
        keys.iter().map(|k| k.to_string()).collect()
    }

    fn keymap() -> Vec<Binding> {
        vec![
            b(&["cmd-t"], "gentle::NewTab", None),
            b(&["ctrl-space", "h"], "gentle::FocusPaneLeft", None),
            b(&["ctrl-space", "c"], "gentle::NewTab", None),
            b(&["ctrl-space", ":"], "gentle::OpenPalette", None),
            b(&["ctrl-space", "shift-;"], "gentle::OpenPalette", None),
            b(&["ctrl-space", "x"], "text_input::Unknown", None),
            b(
                &["g", "g"],
                "changes::SelectFirst",
                Some("GentleChanges && vim"),
            ),
            b(&["j"], "changes::SelectNext", Some("GentleChanges && vim")),
            b(
                &["escape"],
                "gentle::ExitPaneMode",
                Some("GentleWorkspace && pane_mode"),
            ),
            b(
                &["h"],
                "gentle::FocusPaneLeft",
                Some("GentleWorkspace && pane_mode"),
            ),
            b(
                &["escape"],
                "session::Abort",
                Some("GentleSession && !pane_mode"),
            ),
        ]
    }

    #[test]
    fn leader_section_ignores_how_the_leader_is_spelled() {
        let keymap = [b(&["ctrl-shift-a", "h"], "gentle::FocusPaneLeft", None)];
        for leader in ["ctrl-shift-a", "shift-ctrl-a"] {
            let groups = groups(&keymap, leader);
            assert_eq!(groups.len(), 1, "leader {leader}");
            assert_eq!(groups[0].section, Section::Leader, "leader {leader}");
        }
        let keymap = [b(&["super-a", "h"], "gentle::FocusPaneLeft", None)];
        assert_eq!(groups(&keymap, "cmd-a")[0].section, Section::Leader);
        assert_eq!(groups(&keymap, "ctrl-a")[0].section, Section::Direct);
    }

    #[test]
    fn same_keys_compares_sequences_and_the_minus_key() {
        assert!(same_keys("ctrl-space h", "ctrl-space  h"));
        assert!(!same_keys("ctrl-space h", "ctrl-space"));
        assert!(same_keys("cmd--", "cmd--"));
        assert!(!same_keys("cmd--", "cmd-="));
    }

    #[test]
    fn hints_list_the_keys_that_continue_the_pending_prefix() {
        let hints = hints(&keymap(), &pending(&["ctrl-space"]));
        let labels: Vec<_> = hints
            .iter()
            .map(|h| (h.keys[0].as_str(), h.label))
            .collect();
        assert_eq!(
            labels,
            [
                ("h", "Focus pane left"),
                ("c", "New agent tab"),
                (":", "Command palette"),
            ]
        );
    }

    #[test]
    fn hints_merge_aliases_of_one_action_and_prefer_the_glyph() {
        let hints = hints(&keymap(), &pending(&["ctrl-space"]));
        let palette = hints.iter().find(|h| h.label == "Command palette").unwrap();
        assert_eq!(palette.keys, [":"]);
        assert!(!hints.iter().any(|h| h.keys.iter().any(|k| k == "shift-;")));
        assert!(!hints.iter().any(|h| h.keys.iter().any(|k| k == "shift-/")));
    }

    #[test]
    fn hints_are_empty_when_nothing_continues_the_prefix() {
        assert!(hints(&keymap(), &pending(&["ctrl-q"])).is_empty());
        // A complete binding is not a prefix of itself.
        assert!(hints(&keymap(), &pending(&["ctrl-space", "h"])).is_empty());
    }

    #[test]
    fn hints_follow_multi_step_prefixes_and_keep_the_rest_of_the_sequence() {
        let hints = hints(&keymap(), &pending(&["g"]));
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].keys, ["g"]);
        assert_eq!(hints[0].label, "Changes · first");
        let long = [b(&["ctrl-a", "w", "x"], "gentle::ClosePane", None)];
        let hints = super::hints(&long, &pending(&["ctrl-a"]));
        assert_eq!(hints[0].keys, ["w x"]);
    }

    #[test]
    fn hints_skip_unlabelled_actions() {
        let hints = hints(&keymap(), &pending(&["ctrl-space"]));
        assert!(hints.iter().all(|h| h.keys[0] != "x"));
    }

    #[test]
    fn groups_come_in_the_documented_order_and_skip_empty_ones() {
        let groups = groups(&keymap(), "ctrl-space");
        let sections: Vec<_> = groups.iter().map(|g| g.section).collect();
        assert_eq!(
            sections,
            [
                Section::Direct,
                Section::Leader,
                Section::PaneMode,
                Section::Files,
                Section::Session
            ]
        );
        let only_direct = super::groups(&keymap()[..1], "ctrl-space");
        assert_eq!(only_direct.len(), 1);
        assert_eq!(only_direct[0].section, Section::Direct);
    }

    #[test]
    fn rows_are_alphabetical_merge_aliases_and_mark_vim_keys() {
        let groups = groups(&keymap(), "ctrl-space");
        let leader = groups
            .iter()
            .find(|g| g.section == Section::Leader)
            .unwrap();
        let labels: Vec<_> = leader.rows.iter().map(|r| r.label).collect();
        assert_eq!(
            labels,
            ["Command palette", "Focus pane left", "New agent tab"]
        );
        assert_eq!(leader.rows[0].keys, ["ctrl-space :"]);
        let files = groups.iter().find(|g| g.section == Section::Files).unwrap();
        assert!(files.rows.iter().all(|r| r.vim));
        let direct = groups
            .iter()
            .find(|g| g.section == Section::Direct)
            .unwrap();
        assert!(direct.rows.iter().all(|r| !r.vim));
    }

    #[test]
    fn same_action_in_two_sections_is_listed_in_each() {
        let groups = groups(&keymap(), "ctrl-space");
        let count = groups
            .iter()
            .filter(|g| g.rows.iter().any(|r| r.label == "New agent tab"))
            .count();
        assert_eq!(count, 2);
    }

    #[test]
    fn search_matches_label_keys_and_section_case_insensitively() {
        let all = groups(&keymap(), "ctrl-space");
        let by_label = search(&all, "PALETTE", true);
        assert_eq!(by_label.len(), 1);
        assert_eq!(by_label[0].rows.len(), 1);
        let by_keys = search(&all, "⌘T", true);
        assert_eq!(by_keys[0].rows[0].label, "New agent tab");
        let by_section = search(&all, "leader", true);
        assert_eq!(by_section.len(), 1);
        assert_eq!(by_section[0].rows.len(), 3);
        // Every word has to match.
        assert_eq!(search(&all, "leader palette", true)[0].rows.len(), 1);
    }

    #[test]
    fn search_with_no_hit_is_empty_and_blank_keeps_everything() {
        let all = groups(&keymap(), "ctrl-space");
        assert!(search(&all, "zzz", true).is_empty());
        assert_eq!(search(&all, "   ", true), all);
    }

    #[test]
    fn workspace_action_labels_are_renamed() {
        assert_eq!(label("gentle::CreateWorkspace"), Some("Create workspace"));
        assert_eq!(label("gentle::DeleteWorkspace"), Some("Delete workspace…"));
    }

    #[test]
    fn preferred_display_sequence_chooses_the_glyph_alias() {
        let bindings = [
            b(&["ctrl-space", "?"], "gentle::OpenShortcuts", None),
            b(&["ctrl-space", "shift-/"], "gentle::OpenShortcuts", None),
            b(&["ctrl-space", ":"], "gentle::OpenPalette", None),
            b(&["ctrl-space", "shift-;"], "gentle::OpenPalette", None),
        ];
        assert_eq!(
            preferred_display_sequence("gentle::OpenShortcuts", "ctrl-space shift-/", &bindings),
            Some("ctrl-space ?".into())
        );
        assert_eq!(
            preferred_display_sequence("gentle::OpenPalette", "ctrl-space shift-;", &bindings),
            Some("ctrl-space :".into())
        );
        assert!(
            preferred_display_sequence("gentle::OpenShortcuts", "ctrl-space ?", &bindings)
                .is_none()
        );
    }
}
