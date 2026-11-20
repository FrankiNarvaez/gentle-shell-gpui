//! What an agent may set on a diagram element to style it, the way Eraser's
//! diagrams take `color` and `icon`: a color from a small named palette (the
//! app draws each in its theme's tone) and an icon from a curated set of
//! Lucide names. Unknown values are dropped, not errors: the shape still
//! draws in its kind's look.

use serde_json::Value;

/// A named color an element may take; the app maps each to a theme tone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tint {
    Blue,
    Green,
    Yellow,
    Red,
    Purple,
    Teal,
    Pink,
    Gold,
    Gray,
}

impl Tint {
    pub const ALL: [Self; 9] = [
        Self::Blue,
        Self::Green,
        Self::Yellow,
        Self::Red,
        Self::Purple,
        Self::Teal,
        Self::Pink,
        Self::Gold,
        Self::Gray,
    ];

    /// The wire name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Blue => "blue",
            Self::Green => "green",
            Self::Yellow => "yellow",
            Self::Red => "red",
            Self::Purple => "purple",
            Self::Teal => "teal",
            Self::Pink => "pink",
            Self::Gold => "gold",
            Self::Gray => "gray",
        }
    }

    /// A wire name or a common synonym (`orange`, `cyan`, `violet`, `grey`),
    /// any case.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        let named = Self::ALL.into_iter().find(|tint| tint.name() == s);
        named.or(match s.as_str() {
            "orange" | "amber" => Some(Self::Yellow),
            "violet" | "indigo" => Some(Self::Purple),
            "cyan" | "aqua" => Some(Self::Teal),
            "magenta" | "rose" => Some(Self::Pink),
            "grey" | "slate" | "neutral" => Some(Self::Gray),
            _ => None,
        })
    }

    /// `color` of a raw element.
    pub fn field(raw: &Value) -> Option<Self> {
        raw.get("color")
            .and_then(Value::as_str)
            .and_then(Self::parse)
    }

    /// The tint for the `i`-th element when none is set: a fixed cycle, so
    /// tables or timeline groups read apart without the agent choosing.
    pub fn cycle(i: usize) -> Self {
        const CYCLE: [Tint; 6] = [
            Tint::Blue,
            Tint::Green,
            Tint::Purple,
            Tint::Teal,
            Tint::Yellow,
            Tint::Pink,
        ];
        CYCLE[i % CYCLE.len()]
    }
}

/// The Lucide icons an element may name (`icon`), all bundled with the app.
pub const ICONS: &[&str] = &[
    "activity",
    "archive",
    "book-open",
    "bot",
    "box",
    "boxes",
    "brain",
    "bug",
    "building",
    "cable",
    "calendar",
    "chart-column",
    "circle-check",
    "circle-x",
    "clock",
    "cloud",
    "cloud-cog",
    "cloud-download",
    "cloud-upload",
    "code",
    "container",
    "cookie",
    "cpu",
    "credit-card",
    "database",
    "database-zap",
    "download",
    "eye",
    "file",
    "file-text",
    "fingerprint-pattern",
    "flag",
    "folder",
    "gauge",
    "git-branch",
    "git-merge",
    "git-pull-request",
    "globe",
    "graduation-cap",
    "hammer",
    "hard-drive",
    "heart",
    "hourglass",
    "image",
    "inbox",
    "key-round",
    "laptop",
    "layers",
    "lightbulb",
    "list-todo",
    "lock",
    "log-in",
    "log-out",
    "mail",
    "map-pin",
    "memory-stick",
    "merge",
    "message-square",
    "monitor",
    "network",
    "package",
    "pause",
    "play",
    "plug",
    "puzzle",
    "radio",
    "receipt",
    "refresh-cw",
    "repeat",
    "rocket",
    "router",
    "satellite",
    "scale",
    "scan-face",
    "search",
    "send",
    "server",
    "server-cog",
    "settings",
    "shield",
    "shield-check",
    "shopping-cart",
    "smartphone",
    "sparkles",
    "split",
    "square-function",
    "star",
    "store",
    "table",
    "terminal",
    "timer",
    "trash",
    "triangle-alert",
    "truck",
    "unplug",
    "upload",
    "user",
    "users",
    "wallet",
    "warehouse",
    "webhook",
    "wifi",
    "workflow",
    "wrench",
    "zap",
];

/// An icon name as [`ICONS`] spells it, from what an agent wrote: any case,
/// spaces or underscores for dashes, and a few common names
/// (`lambda`, `queue`, `api`, `user-group`, `fingerprint`).
pub fn icon(name: &str) -> Option<&'static str> {
    let name = name.trim().to_ascii_lowercase().replace([' ', '_'], "-");
    let alias = match name.as_str() {
        "lambda" | "function" | "fn" => "square-function",
        "queue" | "stream" | "kafka" => "layers",
        "api" | "service" => "server",
        "cache" | "redis" => "zap",
        "db" | "postgres" | "sql" => "database",
        "user-group" | "team" | "people" => "users",
        "person" | "actor" => "user",
        "fingerprint" | "auth" => "fingerprint-pattern",
        "phone" | "mobile" => "smartphone",
        "web" | "browser" | "internet" => "globe",
        "storage" | "bucket" | "s3" => "archive",
        "email" => "mail",
        "chat" | "message" => "message-square",
        "payment" | "payments" | "card" => "credit-card",
        "cart" => "shopping-cart",
        "k8s" | "kubernetes" | "docker" => "container",
        "ai" | "llm" | "model" => "sparkles",
        "worker" | "job" | "cron" => "timer",
        "alert" | "warning" => "triangle-alert",
        "ok" | "done" | "check" => "circle-check",
        "error" | "fail" | "failed" => "circle-x",
        other => other,
    };
    ICONS.iter().copied().find(|known| *known == alias)
}

/// `icon` of a raw element, as [`icon`] reads it.
pub fn icon_field(raw: &Value) -> Option<&'static str> {
    raw.get("icon").and_then(Value::as_str).and_then(icon)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn colors_take_names_and_synonyms() {
        assert_eq!(Tint::parse("Blue"), Some(Tint::Blue));
        assert_eq!(Tint::parse(" orange "), Some(Tint::Yellow));
        assert_eq!(Tint::parse("grey"), Some(Tint::Gray));
        assert_eq!(Tint::parse("#ff0000"), None);
        assert_eq!(Tint::field(&json!({"color": "cyan"})), Some(Tint::Teal));
        for tint in Tint::ALL {
            assert_eq!(Tint::parse(tint.name()), Some(tint));
        }
    }

    #[test]
    fn icons_take_lucide_names_and_common_aliases() {
        assert_eq!(icon("Credit Card"), Some("credit-card"));
        assert_eq!(icon("lambda"), Some("square-function"));
        assert_eq!(icon("kubernetes"), Some("container"));
        assert_eq!(icon("not-an-icon"), None);
        assert_eq!(icon_field(&json!({"icon": "database"})), Some("database"));
    }
}
