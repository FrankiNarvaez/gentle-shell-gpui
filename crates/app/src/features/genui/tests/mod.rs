//! Interaction tests: real surfaces in headless GPUI windows, driven by
//! native pointer and key events like a person would, with a stand-in for
//! the conversation recording what each surface asks of it.

use gentle_core::genui;
use serde_json::Value;

use super::diagram::DiagramEvent;

mod diagram;
mod panels;

/// What a surface asked the conversation for.
#[derive(Debug, PartialEq)]
enum Seen {
    Send(String),
    Ask(String),
    Expand,
    Collapse,
    OpenWeb(String),
    CancelSubagent(String),
}

impl From<&DiagramEvent> for Seen {
    fn from(event: &DiagramEvent) -> Self {
        match event {
            DiagramEvent::Send(command) => Seen::Send(command.clone()),
            DiagramEvent::Ask(text) => Seen::Ask(text.clone()),
            DiagramEvent::Expand => Seen::Expand,
            DiagramEvent::Collapse => Seen::Collapse,
            DiagramEvent::OpenWeb(url) => Seen::OpenWeb(url.clone()),
            DiagramEvent::CancelSubagent(id) => Seen::CancelSubagent(id.clone()),
        }
    }
}

/// A `/gentle-ui:event` command's JSON.
fn event_json(command: &str) -> Value {
    let json = command
        .strip_prefix(&format!("/{} ", genui::EVENT_COMMAND))
        .expect("an event command");
    serde_json::from_str(json).expect("one line of JSON")
}

/// The ops of an event's changes, in order.
fn ops(event: &Value) -> Vec<&str> {
    event["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["op"].as_str().unwrap())
        .collect()
}
