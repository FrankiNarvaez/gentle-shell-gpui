//! Input style (Configuration › Appearance): how every agent tab's composer
//! looks, saved in the app's `settings.json`
//! (`gentle_core::input_style::SETTINGS_KEY`).

use gentle_core::input_style::{InputStyle, SETTINGS_KEY};
use gpui_kit::{App, Context, Global, Subscription};

struct InputStyleChoice(InputStyle);

impl Global for InputStyleChoice {}

/// Read the saved choice.
pub fn init(cx: &mut App) {
    let saved = crate::shared::themes::store_path()
        .and_then(|path| crate::shared::themes::read_key(&path, SETTINGS_KEY))
        .map(|value| InputStyle::from_json(&value))
        .unwrap_or_default();
    cx.set_global(InputStyleChoice(saved));
}

pub fn style(cx: &App) -> InputStyle {
    cx.try_global::<InputStyleChoice>()
        .map_or_else(InputStyle::default, |s| s.0)
}

/// Redraw `T` when the style changes: panes are cached views, so a window
/// refresh alone doesn't draw an open tab again.
pub fn follow<T: 'static>(cx: &mut Context<T>) -> Subscription {
    cx.observe_global::<InputStyleChoice>(|_, cx| cx.notify())
}

#[cfg(test)]
pub(super) fn set_test_style(style: InputStyle, cx: &mut App) {
    cx.set_global(InputStyleChoice(style));
}

/// Choose and save the style. Written at once (a click, a small file), so
/// writes can't land out of order.
pub fn set_style(style: InputStyle, cx: &mut App) {
    cx.set_global(InputStyleChoice(style));
    if let Some(path) = crate::shared::themes::store_path()
        && let Err(err) = crate::shared::themes::write_key(&path, SETTINGS_KEY, style.to_json())
    {
        eprintln!("gentle-shell-gpui: could not save the input style: {err}");
    }
}
