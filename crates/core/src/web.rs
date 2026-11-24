//! URLs the app can show in its in-app browser. The auto-open path only
//! follows loopback servers printed by a dev command; the explicit
//! `gentle-open` tool and `/web` accept any http(s) URL.

/// The first `http(s)` URL in `text`, trimmed of the punctuation logs add.
pub fn http_url(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let mut best: Option<(usize, &'static str)> = None;
    for scheme in ["http://", "https://"] {
        if let Some(at) = lower.find(scheme)
            && best.is_none_or(|(best_at, _)| at < best_at)
        {
            best = Some((at, scheme));
        }
    }
    let (at, scheme) = best?;
    let rest = &text[at..];
    let end = rest
        .char_indices()
        .find(|(_, c)| {
            c.is_whitespace()
                || c.is_control()
                || matches!(
                    c,
                    '"' | '\'' | '`' | '<' | '>' | '(' | ')' | '{' | '}' | ',' | ';'
                )
        })
        .map(|(index, _)| index)
        .unwrap_or(rest.len());
    let mut url = rest[..end]
        .trim_end_matches(['.', ',', ':', ';', '!', '?'])
        .to_string();
    // A log may wrap the URL in brackets (`[http://localhost:3000]`), but an
    // IPv6 host keeps its own closing bracket (`http://[::1]`).
    if url.ends_with(']') {
        let authority = url.get(scheme.len()..).unwrap_or_default();
        if !authority.starts_with('[') || authority.matches(']').count() > 1 {
            url.pop();
        }
    }
    let authority = url.get(scheme.len()..)?;
    let host = authority.split(['/', '?', '#']).next()?;
    if host.is_empty() || host.contains('@') {
        return None;
    }
    Some(url)
}

/// The first local development URL (`localhost`, `127.0.0.1` or `[::1]`) in
/// `text`. Only these are opened automatically; an arbitrary URL needs the
/// explicit tool.
pub fn loopback_url(text: &str) -> Option<String> {
    let url = http_url(text)?;
    let scheme_len = if url
        .get(..8)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
    {
        8
    } else {
        7
    };
    let authority = url.get(scheme_len..)?;
    let hostport = authority.split(['/', '?', '#']).next()?;
    let host = if let Some(rest) = hostport.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else {
        hostport.split(':').next().unwrap_or(hostport)
    };
    matches!(
        host.to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "::1"
    )
    .then_some(url)
}

/// Whether two URLs show the same page, so one browser card serves both: the
/// same scheme, host and port (any letter case) and the same path, query and
/// fragment, a trailing `/` aside.
pub fn same_page(a: &str, b: &str) -> bool {
    fn key(url: &str) -> (String, &str) {
        let url = url.trim().trim_end_matches('/');
        let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
        let at = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (host, tail) = rest.split_at(at);
        (
            format!(
                "{}://{}",
                scheme.to_ascii_lowercase(),
                host.to_ascii_lowercase()
            ),
            tail,
        )
    }
    key(a) == key(b)
}

#[cfg(test)]
mod tests {
    use super::{http_url, loopback_url, same_page};

    #[test]
    fn one_page_whatever_the_trailing_slash_or_host_case() {
        let canvas = "http://127.0.0.1:59156/fGw3aH90/";
        assert!(same_page(canvas, "http://127.0.0.1:59156/fGw3aH90"));
        assert!(same_page("HTTP://LocalHost:3000/", "http://localhost:3000"));
        assert!(same_page(
            "http://localhost:3000/a?b=1",
            "http://localhost:3000/a?b=1"
        ));
        // Another token, port, path or query is another page.
        assert!(!same_page(canvas, "http://127.0.0.1:59156/other/"));
        assert!(!same_page(canvas, "http://127.0.0.1:59157/fGw3aH90/"));
        assert!(!same_page(
            "http://localhost:3000/a",
            "http://localhost:3000/A"
        ));
        assert!(!same_page(
            "http://localhost:3000/a?b=1",
            "http://localhost:3000/a?b=2"
        ));
        assert!(!same_page(
            "http://localhost:3000/",
            "https://localhost:3000/"
        ));
    }

    #[test]
    fn finds_a_dev_server_url_in_log_lines() {
        assert_eq!(
            loopback_url("  ➜  Local:   http://localhost:5173/  ").as_deref(),
            Some("http://localhost:5173/")
        );
        assert_eq!(
            loopback_url("\u{1b}[32mhttp://127.0.0.1:3000\u{1b}[0m").as_deref(),
            Some("http://127.0.0.1:3000")
        );
        assert_eq!(
            loopback_url("ready on (http://localhost:8080)").as_deref(),
            Some("http://localhost:8080")
        );
    }

    #[test]
    fn keeps_paths_and_query_strings() {
        assert_eq!(
            loopback_url("http://127.0.0.1:3000/app?tab=1,").as_deref(),
            Some("http://127.0.0.1:3000/app?tab=1")
        );
        assert_eq!(
            loopback_url("https://[::1]:8443/login").as_deref(),
            Some("https://[::1]:8443/login")
        );
    }

    #[test]
    fn takes_the_first_url_of_the_output() {
        assert_eq!(
            loopback_url("local http://localhost:3000 network http://localhost:4000").as_deref(),
            Some("http://localhost:3000")
        );
    }

    #[test]
    fn only_loopback_hosts_open_automatically() {
        assert_eq!(
            http_url("https://example.com/app").as_deref(),
            Some("https://example.com/app")
        );
        assert_eq!(loopback_url("https://example.com/app"), None);
        assert_eq!(loopback_url("http://localhost.evil.com"), None);
        assert_eq!(loopback_url("http://user@localhost:3000"), None);
        assert_eq!(loopback_url("localhost:3000"), None);
        assert_eq!(loopback_url("http://"), None);
    }

    #[test]
    fn accepts_a_bare_loopback_host() {
        assert_eq!(
            loopback_url("http://localhost").as_deref(),
            Some("http://localhost")
        );
        assert_eq!(
            loopback_url("http://[::1]/").as_deref(),
            Some("http://[::1]/")
        );
    }
}
