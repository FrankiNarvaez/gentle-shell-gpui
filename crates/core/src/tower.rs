//! What `gentle-shell tower` prints once it serves. From Gentle Shell 5.0.0,
//! `--json` prints one `gentle-shell.tower/v1` line
//! (`{"schema","url","host","port"}`) instead of the text; older Shells
//! print `Gentle Tower: <url>` and refuse the flag at argument parsing
//! (`gentle-shell tower: unknown tower argument "--json"` on stderr, exit 2)
//! before starting anything.

use serde_json::Value;

/// The schema of `gentle-shell tower --json`'s line.
pub const JSON_SCHEMA: &str = "gentle-shell.tower/v1";

/// The text line an older Shell (or one run without `--json`) prints.
const TEXT_READY: &str = "Gentle Tower: ";

/// The page's URL from one output line: the `--json` line of the known
/// schema, or the text line. Any other line is not the ready signal.
pub fn ready_url(line: &str) -> Option<String> {
    let line = line.trim();
    if line.starts_with('{') {
        let record: Value = serde_json::from_str(line).ok()?;
        if record.get("schema")?.as_str()? != JSON_SCHEMA {
            return None;
        }
        let url = record.get("url")?.as_str()?.trim();
        return (!url.is_empty()).then(|| url.to_string());
    }
    let url = line.strip_prefix(TEXT_READY)?.trim();
    (!url.is_empty()).then(|| url.to_string())
}

/// Whether a Shell that ended without serving refused `--json` as an
/// unknown argument (older than 5.0.0): run it again without the flag.
pub fn refused_json_flag(stderr: &str) -> bool {
    stderr.contains("unknown tower argument \"--json\"")
}

#[cfg(test)]
mod tests {
    use super::{ready_url, refused_json_flag};

    #[test]
    fn the_json_line_names_the_url() {
        // As `tests/tower-open.test.ts` upstream prints it.
        let line = r#"{"schema":"gentle-shell.tower/v1","url":"http://127.0.0.1:47321/","host":"127.0.0.1","port":47321}"#;
        assert_eq!(ready_url(line).as_deref(), Some("http://127.0.0.1:47321/"));
        assert_eq!(
            ready_url(&format!("{line}\n")).as_deref(),
            Some("http://127.0.0.1:47321/")
        );
    }

    #[test]
    fn other_json_is_not_the_ready_line() {
        for line in [
            r#"{"schema":"gentle-shell.tower/v2","url":"http://127.0.0.1:1/"}"#,
            r#"{"url":"http://127.0.0.1:1/"}"#,
            r#"{"schema":"gentle-shell.tower/v1","url":""}"#,
            r#"{"schema":"gentle-shell.tower/v1","url":7}"#,
            r#"{"schema":"gentle-shell.tower/v1""#,
        ] {
            assert_eq!(ready_url(line), None, "{line}");
        }
    }

    #[test]
    fn the_text_line_of_older_shells_still_names_the_url() {
        assert_eq!(
            ready_url("Gentle Tower: http://127.0.0.1:47321/\n").as_deref(),
            Some("http://127.0.0.1:47321/")
        );
        assert_eq!(ready_url("Building the web app…"), None);
        assert_eq!(ready_url("Gentle Tower: "), None);
    }

    #[test]
    fn only_a_refused_json_flag_falls_back() {
        // Gentle Shell 4.9.0's `fail(...)` for the flag it does not know.
        assert!(refused_json_flag(
            "gentle-shell tower: unknown tower argument \"--json\"\nRun 'gentle-shell --help' for usage.\n"
        ));
        assert!(!refused_json_flag(
            "gentle-shell tower: unknown tower argument \"--port\""
        ));
        assert!(!refused_json_flag("EADDRINUSE: address already in use"));
    }
}
