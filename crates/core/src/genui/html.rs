//! The `html` kind: a page the agent writes, shown in an isolated web view
//! that talks to the app with the MCP Apps protocol (see [`super::apps`]).
//! This module reads its spec and builds the Content-Security-Policy the page
//! runs under: no network, no nested frames and no remote resources unless
//! the spec declares the origins, inline scripts and styles allowed.

use serde_json::{Map, Value};

/// The largest page accepted, in bytes.
pub const MAX_HTML: usize = 512 * 1024;
/// Preferred heights outside this range (px) are clamped.
pub const MIN_HEIGHT: f64 = 120.;
pub const MAX_HEIGHT: f64 = 2000.;
/// At most this many origins per CSP list.
const MAX_ORIGINS: usize = 20;

/// Origins the page may reach (MCP Apps `McpUiResourceCsp`), each already
/// checked by [`origin_source`]. Empty lists keep the restrictive defaults.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Csp {
    /// fetch/XHR/WebSocket (`connect-src`).
    pub connect: Vec<String>,
    /// Scripts, styles, images, fonts and media.
    pub resources: Vec<String>,
    /// Nested iframes (`frame-src`).
    pub frames: Vec<String>,
    /// `<base href>` (`base-uri`).
    pub base_uris: Vec<String>,
}

impl Csp {
    /// The keys of `value` (an MCP Apps `csp` object); unknown keys are
    /// ignored, a bad origin is an error.
    pub fn parse(value: &Value) -> Result<Self, Vec<String>> {
        let mut errors = Vec::new();
        let Some(object) = value.as_object() else {
            if !value.is_null() {
                errors.push("spec.csp must be an object".to_string());
            }
            return if errors.is_empty() {
                Ok(Self::default())
            } else {
                Err(errors)
            };
        };
        let mut list = |key: &str| -> Vec<String> {
            let Some(items) = object.get(key) else {
                return Vec::new();
            };
            let Some(items) = items.as_array() else {
                errors.push(format!("spec.csp.{key} must be an array of origins"));
                return Vec::new();
            };
            if items.len() > MAX_ORIGINS {
                errors.push(format!(
                    "spec.csp.{key} lists {} origins, at most {MAX_ORIGINS}",
                    items.len()
                ));
            }
            let mut origins = Vec::new();
            for item in items.iter().take(MAX_ORIGINS) {
                match item.as_str().and_then(origin_source) {
                    Some(origin) if !origins.contains(&origin) => origins.push(origin),
                    Some(_) => {}
                    None => errors.push(format!(
                        "spec.csp.{key}: {item} is not an origin like https://api.example.com"
                    )),
                }
            }
            origins
        };
        let csp = Self {
            connect: list("connectDomains"),
            resources: list("resourceDomains"),
            frames: list("frameDomains"),
            base_uris: list("baseUriDomains"),
        };
        if errors.is_empty() {
            Ok(csp)
        } else {
            Err(errors)
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Back to the MCP Apps shape (for the host capabilities' `sandbox.csp`).
    pub fn to_value(&self) -> Value {
        let mut object = Map::new();
        for (key, list) in [
            ("connectDomains", &self.connect),
            ("resourceDomains", &self.resources),
            ("frameDomains", &self.frames),
            ("baseUriDomains", &self.base_uris),
        ] {
            if !list.is_empty() {
                object.insert(key.into(), list.clone().into());
            }
        }
        Value::Object(object)
    }

    /// The policy for the page, per the MCP Apps host rules: the declared
    /// origins and nothing else; with none declared, exactly the spec's
    /// restrictive default (plus `data:`/`blob:` media, `object-src` and
    /// `form-action` closed).
    pub fn policy(&self) -> String {
        let with = |base: &str, extra: &[String]| {
            let mut out = base.to_string();
            for origin in extra {
                out.push(' ');
                out.push_str(origin);
            }
            out
        };
        let or_none = |list: &[String], fallback: &str| {
            if list.is_empty() {
                fallback.to_string()
            } else {
                list.join(" ")
            }
        };
        [
            "default-src 'none'".to_string(),
            with("script-src 'self' 'unsafe-inline'", &self.resources),
            with("style-src 'self' 'unsafe-inline'", &self.resources),
            with("img-src 'self' data: blob:", &self.resources),
            with("font-src 'self' data:", &self.resources),
            with("media-src 'self' data: blob:", &self.resources),
            format!("connect-src {}", or_none(&self.connect, "'none'")),
            format!("frame-src {}", or_none(&self.frames, "'none'")),
            "object-src 'none'".to_string(),
            format!("base-uri {}", or_none(&self.base_uris, "'self'")),
            "form-action 'none'".to_string(),
        ]
        .join("; ")
    }

    /// Whether a frame may navigate to `url`: one of the declared frame
    /// origins (the host's own pages are allowed separately).
    pub fn allows_frame(&self, url: &str) -> bool {
        self.frames.iter().any(|origin| origin_matches(origin, url))
    }
}

/// One `html` spec: `{ html, height?, csp?, data? }`.
#[derive(Debug, Clone, PartialEq)]
pub struct HtmlSpec {
    pub html: String,
    /// The height the page prefers (px), until it reports its own size.
    pub height: Option<f64>,
    pub csp: Csp,
    /// What the page gets as the tool's input and `structuredContent`; an
    /// object (empty when the spec has none).
    pub data: Value,
}

impl HtmlSpec {
    pub fn parse(spec: &Value) -> Result<Self, Vec<String>> {
        let Some(object) = spec.as_object() else {
            return Err(vec!["spec must be an object with an html string".into()]);
        };
        let mut errors = Vec::new();
        let html = match object.get("html").and_then(Value::as_str) {
            Some(html) if html.trim().is_empty() => {
                errors.push("spec.html is empty".to_string());
                String::new()
            }
            Some(html) if html.len() > MAX_HTML => {
                errors.push(format!(
                    "spec.html is {} KiB, at most {} KiB",
                    html.len().div_ceil(1024),
                    MAX_HTML / 1024
                ));
                String::new()
            }
            Some(html) => html.to_string(),
            None => {
                errors.push("spec.html must be a string with the page".to_string());
                String::new()
            }
        };
        let height = object
            .get("height")
            .and_then(Value::as_f64)
            .filter(|h| h.is_finite())
            .map(|h| h.clamp(MIN_HEIGHT, MAX_HEIGHT));
        let csp = Csp::parse(object.get("csp").unwrap_or(&Value::Null)).unwrap_or_else(|e| {
            errors.extend(e);
            Csp::default()
        });
        let data = match object.get("data") {
            None | Some(Value::Null) => Value::Object(Map::new()),
            Some(data @ Value::Object(_)) => data.clone(),
            Some(_) => {
                errors.push("spec.data must be an object".to_string());
                Value::Object(Map::new())
            }
        };
        if errors.is_empty() {
            Ok(Self {
                html,
                height,
                csp,
                data,
            })
        } else {
            Err(errors)
        }
    }

    /// The page as served: the policy as a `<meta>` first thing in the
    /// document, so it holds even where the response header isn't applied,
    /// then `base_css` (the host's defaults: text color, font, color
    /// scheme), which any style of the page overrides.
    pub fn document(&self, base_css: &str) -> String {
        let policy = self.csp.policy();
        let head = format!(
            r#"<meta http-equiv="Content-Security-Policy" content="{policy}"><style>{}</style>"#,
            base_css.replace("</", "<\\/")
        );
        with_head(&self.html, &head)
    }
}

/// `html` with `head` before any of its content: right after the doctype
/// when there is one (before it the page would fall into quirks mode),
/// else first.
pub fn with_head(html: &str, head: &str) -> String {
    let start = html.len()
        - html
            .trim_start_matches(['\u{feff}', ' ', '\t', '\r', '\n'])
            .len();
    let rest = &html[start..];
    let doctype = rest
        .get(..9)
        .is_some_and(|tag| tag.eq_ignore_ascii_case("<!doctype"));
    match rest.find('>').filter(|_| doctype) {
        Some(end) => {
            let at = start + end + 1;
            format!("{}{head}{}", &html[..at], &html[at..])
        }
        None => format!("{head}{html}"),
    }
}

/// `text` as a CSP source if it is a web origin (optionally with a path):
/// `https://host[:port]`, `wss://…`, a `*.` wildcard subdomain, or plain
/// `http://`/`ws://` on loopback only. Keywords, schemes alone, `*` and
/// anything with quotes, spaces or separators are refused, so a spec can't
/// widen the policy beyond the origins it names.
pub fn origin_source(text: &str) -> Option<String> {
    let text = text.trim().trim_end_matches('/');
    let (scheme, rest) = text.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let (authority, path) = match rest.find('/') {
        Some(at) => rest.split_at(at),
        None => (rest, ""),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    let host = host.to_ascii_lowercase();
    let name = host.strip_prefix("*.").unwrap_or(&host);
    let label_ok = |label: &str| {
        !label.is_empty()
            && label.len() <= 63
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    };
    if !name.split('.').all(label_ok) {
        return None;
    }
    if port.is_some_and(|p| !p.chars().all(|c| c.is_ascii_digit()) || p.parse::<u16>().is_err()) {
        return None;
    }
    let path_ok = path
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/-._~%".contains(c));
    if !path_ok {
        return None;
    }
    let loopback = matches!(name, "localhost" | "127.0.0.1") && name == host;
    let secure = matches!(scheme.as_str(), "https" | "wss");
    let plain = matches!(scheme.as_str(), "http" | "ws");
    // A wildcard needs a registrable name under it (not `*.com`).
    if host.starts_with("*.") && !name.contains('.') {
        return None;
    }
    if !(secure || (plain && loopback)) {
        return None;
    }
    let port = port.map(|p| format!(":{p}")).unwrap_or_default();
    Some(format!("{scheme}://{host}{port}{path}"))
}

/// Whether `url` falls under the CSP source `origin` (scheme, host with an
/// optional `*.` wildcard, port, path prefix).
fn origin_matches(origin: &str, url: &str) -> bool {
    let Some((scheme, rest)) = origin.split_once("://") else {
        return false;
    };
    let Some((url_scheme, url_rest)) = url.split_once("://") else {
        return false;
    };
    if !url_scheme.eq_ignore_ascii_case(scheme) {
        return false;
    }
    let split = |s: &str| -> (String, String) {
        let end = s.find(['/', '?', '#']).unwrap_or(s.len());
        (s[..end].to_ascii_lowercase(), s[end..].to_string())
    };
    let (authority, path) = split(rest);
    let (url_authority, url_path) = split(url_rest);
    let host_ok = match authority.strip_prefix("*.") {
        Some(base) => url_authority
            .strip_suffix(base)
            .is_some_and(|sub| sub.len() > 1 && sub.ends_with('.')),
        None => url_authority == authority,
    };
    host_ok && url_path.starts_with(&path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn spec_reads_html_height_csp_and_data() {
        let spec = HtmlSpec::parse(&json!({
            "html": "<p>hi</p>", "height": 50,
            "csp": {"connectDomains": ["https://api.example.com/"], "other": 1},
            "data": {"n": 1},
        }))
        .unwrap();
        assert_eq!(spec.height, Some(MIN_HEIGHT));
        assert_eq!(spec.csp.connect, vec!["https://api.example.com"]);
        assert_eq!(spec.data, json!({"n": 1}));

        let bare = HtmlSpec::parse(&json!({"html": "x"})).unwrap();
        assert_eq!((bare.height, bare.data), (None, json!({})));
        assert!(bare.csp.is_empty());
    }

    #[test]
    fn spec_errors_are_readable() {
        assert!(HtmlSpec::parse(&json!("<p>")).is_err());
        let errors = HtmlSpec::parse(&json!({
            "html": " ", "data": [1], "csp": {"connectDomains": ["*"], "frameDomains": "x"},
        }))
        .unwrap_err();
        assert_eq!(errors.len(), 4, "{errors:?}");
        let big = "a".repeat(MAX_HTML + 1);
        let errors = HtmlSpec::parse(&json!({ "html": big })).unwrap_err();
        assert_eq!(errors, vec!["spec.html is 513 KiB, at most 512 KiB"]);
    }

    #[test]
    fn origins_are_web_origins_only() {
        let ok = |s: &str| origin_source(s);
        assert_eq!(
            ok("https://cdn.jsdelivr.net").as_deref(),
            Some("https://cdn.jsdelivr.net")
        );
        assert_eq!(
            ok("HTTPS://*.Example.com/").as_deref(),
            Some("https://*.example.com")
        );
        assert_eq!(
            ok("wss://rt.example.com:8443").as_deref(),
            Some("wss://rt.example.com:8443")
        );
        assert_eq!(
            ok("http://localhost:5173").as_deref(),
            Some("http://localhost:5173")
        );
        assert_eq!(
            ok("https://x.dev/lib/v1").as_deref(),
            Some("https://x.dev/lib/v1")
        );
        for bad in [
            "*",
            "https:",
            "'unsafe-eval'",
            "data:",
            "http://example.com",
            "https://*.com",
            "https://a.com; script-src *",
            "https://a.com 'unsafe-eval'",
            "https://a.com:99999",
            "javascript://x",
            "https://a.com/\"",
        ] {
            assert_eq!(ok(bad), None, "{bad}");
        }
    }

    #[test]
    fn default_policy_is_the_spec_default_closed() {
        let policy = Csp::default().policy();
        assert!(policy.starts_with("default-src 'none'; script-src 'self' 'unsafe-inline';"));
        assert!(policy.contains("connect-src 'none'"));
        assert!(policy.contains("frame-src 'none'"));
        assert!(policy.contains("object-src 'none'"));
        assert!(policy.contains("base-uri 'self'"));
        assert!(!policy.contains("https:"));
    }

    #[test]
    fn declared_origins_open_only_their_directives() {
        let csp = Csp::parse(&json!({
            "connectDomains": ["https://api.example.com"],
            "resourceDomains": ["https://cdn.example.com"],
            "frameDomains": ["https://*.youtube.com"],
        }))
        .unwrap();
        let policy = csp.policy();
        assert!(policy.contains("connect-src https://api.example.com;"));
        assert!(policy.contains("script-src 'self' 'unsafe-inline' https://cdn.example.com;"));
        assert!(policy.contains("frame-src https://*.youtube.com;"));
        assert!(csp.allows_frame("https://www.youtube.com/embed/x"));
        assert!(!csp.allows_frame("https://youtube.com.evil.dev/"));
        assert!(!csp.allows_frame("http://www.youtube.com/"));
        assert_eq!(
            csp.to_value()["connectDomains"],
            json!(["https://api.example.com"])
        );
    }

    #[test]
    fn policy_meta_goes_after_the_doctype() {
        let meta = "<meta>";
        assert_eq!(
            with_head("\n<!DOCTYPE html><html>", meta),
            format!("\n<!DOCTYPE html>{meta}<html>")
        );
        assert_eq!(with_head("<div>", meta), format!("{meta}<div>"));
        assert_eq!(
            with_head("<!-- x --><p>", meta),
            format!("{meta}<!-- x --><p>")
        );
        let spec = HtmlSpec::parse(&json!({"html": "<!doctype html><p>"})).unwrap();
        let page = spec.document("body{color:#fff}</style><script>");
        assert!(page.starts_with(
            r#"<!doctype html><meta http-equiv="Content-Security-Policy" content="default-src 'none';"#
        ));
        assert!(page.contains("<style>body{color:#fff}<\\/style><script></style><p>"));
    }
}
