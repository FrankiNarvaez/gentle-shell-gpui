//! Agent UI (Configuration › Agent UI): how readily the agent uses
//! `gentle_ui` ([`Mode`]), saved in the app's `settings.json`
//! (`genuiMode`) and followed by every agent tab: a new tab gets it at
//! launch ([`launch_env`]), an open one a `/gentle-ui:event {"op":"mode"}`
//! when it changes ([`follow`]).

use std::cell::Cell;
use std::ffi::OsString;

use gentle_core::genui::mode::{self, Mode};
use gpui_kit::{App, Context, Global, Subscription};

use super::store;

struct AgentUi(Mode);

impl Global for AgentUi {}

/// Read the saved choice.
pub fn init(cx: &mut App) {
    let saved = store::dir(cx)
        .and_then(|dir| {
            crate::shared::themes::read_key(&dir.join("settings.json"), mode::SETTINGS_KEY)
        })
        .map(|value| Mode::from_json(&value))
        .unwrap_or_default();
    cx.set_global(AgentUi(saved));
}

pub fn mode(cx: &App) -> Mode {
    cx.try_global::<AgentUi>()
        .map_or_else(Mode::default, |m| m.0)
}

/// Choose and save the mode; open tabs following it ([`follow`]) hear of
/// it. Written at once (a click, a small file): background writes could
/// land out of order and keep an older choice.
pub fn set_mode(mode: Mode, cx: &mut App) {
    cx.set_global(AgentUi(mode));
    let Some(path) = store::dir(cx).map(|dir| dir.join("settings.json")) else {
        return;
    };
    if let Err(err) = crate::shared::themes::write_key(&path, mode::SETTINGS_KEY, mode.to_json()) {
        eprintln!("gentle-shell-gpui: could not save Agent UI: {err}");
    }
}

/// The variable a new agent tab starts with, so its extension loads in `mode`.
pub fn launch_env(mode: Mode) -> (String, OsString) {
    (mode::ENV.to_string(), mode.id().into())
}

/// Call `send` with each mode the user picks from now on (not the current
/// one, nor the same one again).
pub fn follow<T: 'static>(
    cx: &mut Context<T>,
    send: impl Fn(&mut T, Mode, &mut Context<T>) + 'static,
) -> Subscription {
    let last = Cell::new(mode(cx));
    cx.observe_global::<AgentUi>(move |this, cx| {
        let now = mode(cx);
        if last.replace(now) != now {
            send(this, now, cx);
        }
    })
}
