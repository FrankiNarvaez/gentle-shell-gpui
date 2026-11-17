//! Agent UI: choosing a mode reaches every open tab that follows it (as
//! the command a tab sends its extension) and is saved for the next start.

use gentle_core::genui::mode::{Mode, SETTINGS_KEY};
// Not a glob: GPUI exports a `test` attribute that would shadow `#[test]`.
use gpui_kit::{AppContext as _, Context, Entity, Subscription, TestAppContext};

use super::super::mode;
use super::super::store::StoreDir;

/// A stand-in for an agent tab: records the commands it would run.
struct Tab {
    sent: Vec<String>,
    _follow: Subscription,
}

impl Tab {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            sent: Vec::new(),
            _follow: mode::follow(cx, |this, mode, _| this.sent.push(mode.command())),
        }
    }
}

fn sent(tab: &Entity<Tab>, cx: &mut TestAppContext) -> Vec<String> {
    tab.read_with(cx, |tab, _| tab.sent.clone())
}

#[gpui::test]
fn a_new_mode_reaches_every_open_tab_once_and_is_saved(cx: &mut TestAppContext) {
    let data = std::env::temp_dir().join(format!("genui-mode-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    cx.update(|cx| {
        cx.set_global(StoreDir(data.clone()));
        mode::init(cx);
    });
    assert_eq!(cx.update(|cx| mode::mode(cx)), Mode::Proactive, "default");

    let first = cx.new(Tab::new);
    let second = cx.new(Tab::new);
    cx.update(|cx| mode::set_mode(Mode::WhenAsked, cx));
    cx.run_until_parked();
    let asked = r#"/gentle-ui:event {"op":"mode","mode":"asked"}"#.to_string();
    assert_eq!(sent(&first, cx), vec![asked.clone()]);
    assert_eq!(sent(&second, cx), vec![asked.clone()]);

    // The same choice again sends nothing; a tab opened now starts in it.
    cx.update(|cx| mode::set_mode(Mode::WhenAsked, cx));
    let late = cx.new(Tab::new);
    cx.update(|cx| mode::set_mode(Mode::Off, cx));
    cx.run_until_parked();
    let off = r#"/gentle-ui:event {"op":"mode","mode":"off"}"#.to_string();
    assert_eq!(sent(&first, cx), vec![asked.clone(), off.clone()]);
    assert_eq!(sent(&late, cx), vec![off]);

    // Saved, and read back at the next start.
    let settings: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(data.join("settings.json")).unwrap())
            .unwrap();
    assert_eq!(settings[SETTINGS_KEY], "off");
    cx.update(|cx| {
        mode::init(cx);
        assert_eq!(mode::mode(cx), Mode::Off);
        assert_eq!(
            mode::launch_env(mode::mode(cx)),
            ("GENTLE_UI_MODE".to_string(), "off".into())
        );
    });
    std::fs::remove_dir_all(data).unwrap();
}
