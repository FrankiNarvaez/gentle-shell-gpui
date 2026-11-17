//! Agent UI: how readily the agent uses `gentle_ui`, the user's choice in
//! Configuration. Saved in the app's `settings.json` ([`SETTINGS_KEY`]),
//! given to each new agent tab as [`ENV`] and sent to open tabs as a
//! `/gentle-ui:event {"op":"mode"}` ([`Mode::command`]); the extension
//! turns it into the tool's rules each turn.

use serde_json::{Value, json};

use super::EVENT_COMMAND;

/// Key of the app's `settings.json`.
pub const SETTINGS_KEY: &str = "genuiMode";
/// The variable the extension reads the mode from when it loads.
pub const ENV: &str = "GENTLE_UI_MODE";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    /// The agent shows a surface on its own when one clearly beats prose.
    #[default]
    Proactive,
    /// Only when the user asks for one (or to update one already open).
    WhenAsked,
    /// The agent has no `gentle_ui` tool.
    Off,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Proactive, Mode::WhenAsked, Mode::Off];

    /// What the extension, the env var and the settings file call it.
    pub fn id(self) -> &'static str {
        match self {
            Mode::Proactive => "proactive",
            Mode::WhenAsked => "asked",
            Mode::Off => "off",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::Proactive => "Proactive",
            Mode::WhenAsked => "When asked",
            Mode::Off => "Off",
        }
    }

    /// An [`Self::id`], any case, surrounding spaces ignored.
    pub fn parse(text: &str) -> Option<Mode> {
        let text = text.trim();
        Self::ALL
            .into_iter()
            .find(|mode| mode.id().eq_ignore_ascii_case(text))
    }

    /// The saved value; anything unreadable is the default.
    pub fn from_json(value: &Value) -> Mode {
        value.as_str().and_then(Self::parse).unwrap_or_default()
    }

    pub fn to_json(self) -> Value {
        Value::from(self.id())
    }

    /// The command that switches an open tab's extension to this mode.
    pub fn command(self) -> String {
        format!(
            "/{EVENT_COMMAND} {}",
            json!({"op": "mode", "mode": self.id()})
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_unknown_values_fall_back_to_proactive() {
        for mode in Mode::ALL {
            assert_eq!(Mode::parse(mode.id()), Some(mode));
            assert_eq!(Mode::from_json(&mode.to_json()), mode);
        }
        assert_eq!(Mode::parse(" Off "), Some(Mode::Off));
        assert_eq!(Mode::parse("ASKED"), Some(Mode::WhenAsked));
        assert_eq!(Mode::parse("when asked"), None);
        assert_eq!(Mode::from_json(&json!("sometimes")), Mode::Proactive);
        assert_eq!(Mode::from_json(&json!(true)), Mode::Proactive);
        assert_eq!(Mode::from_json(&Value::Null), Mode::default());
    }

    #[test]
    fn command_is_one_line_the_extension_reads() {
        assert_eq!(
            Mode::WhenAsked.command(),
            r#"/gentle-ui:event {"op":"mode","mode":"asked"}"#
        );
    }
}
