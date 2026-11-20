//! Where a commit author's picture comes from, as Tern finds it: a GitHub
//! noreply address names the account itself; otherwise GitHub's API says
//! which account wrote the author's latest commit in the repository (when
//! it is on GitHub); otherwise Gravatar, by the address's hash.

/// The size, in px, avatars are asked for (twice the largest drawn, for
/// sharp edges on high-density screens).
pub const SIZE: u32 = 64;

/// The address as services hash and compare it: trimmed, lowercase.
pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// The picture of a GitHub noreply address: `123+name@users.noreply.github.com`
/// names account 123, the older `name@users.noreply.github.com` the login.
pub fn github_noreply_url(email: &str) -> Option<String> {
    let local = normalize_email(email)
        .strip_suffix("@users.noreply.github.com")?
        .to_string();
    if local.is_empty() {
        return None;
    }
    Some(match local.split_once('+') {
        Some((id, _)) if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) => {
            format!("https://avatars.githubusercontent.com/u/{id}?s={SIZE}")
        }
        _ => format!("https://avatars.githubusercontent.com/{local}?s={SIZE}"),
    })
}

/// `(owner, repository)` of a GitHub remote, in the HTTPS or SSH form.
pub fn github_repo(remote: &str) -> Option<(String, String)> {
    let remote = remote.trim();
    let path = remote
        .strip_prefix("https://github.com/")
        .or_else(|| remote.strip_prefix("http://github.com/"))
        .or_else(|| remote.strip_prefix("ssh://git@github.com/"))
        .or_else(|| remote.strip_prefix("git@github.com:"))?;
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, repo) = path.split_once('/')?;
    (!owner.is_empty() && !repo.is_empty() && !repo.contains('/'))
        .then(|| (owner.to_string(), repo.to_string()))
}

/// GitHub's latest commit by `email` in `owner/repo`, whose author account
/// has the picture.
pub fn github_commits_url(owner: &str, repo: &str, email: &str) -> String {
    format!(
        "https://api.github.com/repos/{owner}/{repo}/commits?per_page=1&author={}",
        encode(&normalize_email(email))
    )
}

/// The account picture of the first commit in a commits-API answer.
pub fn parse_commits_avatar(json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let url = value.get(0)?.get("author")?.get("avatar_url")?.as_str()?;
    url.starts_with("https://").then(|| {
        let joiner = if url.contains('?') { '&' } else { '?' };
        format!("{url}{joiner}s={SIZE}")
    })
}

/// Gravatar's picture for the address's SHA-256 (hex); 404 when it has none.
pub fn gravatar_url(email_sha256_hex: &str) -> String {
    format!("https://www.gravatar.com/avatar/{email_sha256_hex}?d=404&s={SIZE}")
}

/// Percent-encode what isn't unreserved in a query value.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noreply_addresses_name_the_account() {
        assert_eq!(
            github_noreply_url("  12345+Maya@users.noreply.github.com ").as_deref(),
            Some("https://avatars.githubusercontent.com/u/12345?s=64")
        );
        assert_eq!(
            github_noreply_url("maya@users.noreply.github.com").as_deref(),
            Some("https://avatars.githubusercontent.com/maya?s=64")
        );
        assert_eq!(github_noreply_url("maya@example.com"), None);
        assert_eq!(github_noreply_url("@users.noreply.github.com"), None);
    }

    #[test]
    fn github_remotes_in_every_form() {
        let want = Some(("Gentleman-Programming".into(), "gentle-shell-gpui".into()));
        for remote in [
            "https://github.com/Gentleman-Programming/gentle-shell-gpui.git",
            "https://github.com/Gentleman-Programming/gentle-shell-gpui/",
            "git@github.com:Gentleman-Programming/gentle-shell-gpui.git",
            "ssh://git@github.com/Gentleman-Programming/gentle-shell-gpui",
        ] {
            assert_eq!(github_repo(remote), want, "{remote}");
        }
        assert_eq!(github_repo("https://gitlab.com/a/b.git"), None);
        assert_eq!(github_repo("https://github.com/only-owner"), None);
    }

    #[test]
    fn the_commits_answer_gives_the_author_picture() {
        let json = r#"[{"sha":"a","author":{"login":"maya","avatar_url":"https://avatars.githubusercontent.com/u/9?v=4"}}]"#;
        assert_eq!(
            parse_commits_avatar(json).as_deref(),
            Some("https://avatars.githubusercontent.com/u/9?v=4&s=64")
        );
        // A commit GitHub can't tie to an account has `"author": null`.
        assert_eq!(parse_commits_avatar(r#"[{"sha":"a","author":null}]"#), None);
        assert_eq!(parse_commits_avatar("[]"), None);
        assert_eq!(parse_commits_avatar("not json"), None);
    }

    #[test]
    fn the_author_query_is_encoded() {
        assert_eq!(
            github_commits_url("o", "r", "Maya+x@Example.com"),
            "https://api.github.com/repos/o/r/commits?per_page=1&author=maya%2Bx%40example.com"
        );
    }
}
