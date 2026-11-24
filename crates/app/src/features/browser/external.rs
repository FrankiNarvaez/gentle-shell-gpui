//! Pages outside the app: where the pages agents open go (Configuration ›
//! Devices › Browser, "Open pages") and which browser opens a page outside
//! the app (the card's ↗, and agents' pages when they go to the browser).
//! Both are saved in the app's `settings.json`; the choices themselves are
//! `gentle_core::web_browsers`.

use gentle_core::web_browsers::{self, AgentPages, BROWSER_KEY, PAGES_KEY, WebBrowser};
use gpui_kit::{App, Global};

use crate::platform::browsers;

#[derive(Clone, Copy, Default)]
struct External {
    pages: AgentPages,
    /// `None`: the system's default browser.
    browser: Option<&'static WebBrowser>,
}

impl Global for External {}

fn read(key: &str) -> Option<serde_json::Value> {
    let path = crate::shared::themes::store_path()?;
    crate::shared::themes::read_key(&path, key)
}

fn save(key: &str, value: serde_json::Value, what: &str) {
    if let Some(path) = crate::shared::themes::store_path()
        && let Err(err) = crate::shared::themes::write_key(&path, key, value)
    {
        eprintln!("gentle-shell-gpui: could not save {what}: {err}");
    }
}

/// Read the saved choices.
pub fn init(cx: &mut App) {
    cx.set_global(External {
        pages: read(PAGES_KEY)
            .map(|value| AgentPages::from_json(&value))
            .unwrap_or_default(),
        browser: read(BROWSER_KEY).and_then(|value| web_browsers::choice(&value)),
    });
}

fn current(cx: &App) -> External {
    cx.try_global::<External>().copied().unwrap_or_default()
}

pub fn agent_pages(cx: &App) -> AgentPages {
    current(cx).pages
}

pub fn set_agent_pages(pages: AgentPages, cx: &mut App) {
    cx.set_global(External {
        pages,
        ..current(cx)
    });
    save(PAGES_KEY, pages.to_json(), "where agents' pages open");
}

/// The chosen browser; `None` is the system's default.
pub fn browser(cx: &App) -> Option<&'static WebBrowser> {
    current(cx).browser
}

pub fn set_browser(browser: Option<&'static WebBrowser>, cx: &mut App) {
    cx.set_global(External {
        browser,
        ..current(cx)
    });
    save(
        BROWSER_KEY,
        web_browsers::choice_json(browser),
        "the browser",
    );
}

/// Open `url` outside the app, in the chosen browser while agents' pages go
/// there ([`web_browsers::opener`]). The system's default opens it
/// otherwise, when the chosen one is gone, or when it fails to start.
pub fn open(url: &str, cx: &mut App) {
    let chosen = current(cx);
    let Some(browser) = web_browsers::opener(chosen.pages, chosen.browser) else {
        cx.open_url(url);
        return;
    };
    let url = url.to_string();
    let opened = cx.background_executor().spawn(async move {
        let opened = browsers::locate(browser).is_some_and(|installed| {
            browsers::open(&installed, &url)
                .inspect_err(|err| {
                    eprintln!("gentle-shell-gpui: could not open {}: {err}", browser.name)
                })
                .is_ok()
        });
        (opened, url)
    });
    cx.spawn(async move |cx| {
        let (opened, url) = opened.await;
        if !opened {
            cx.update(|cx| cx.open_url(&url));
        }
    })
    .detach();
}
