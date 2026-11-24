//! Pure decisions for the Gentle Shell setup check and installer.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallAction {
    Npm,
    Upgrade,
}

/// A resolved launcher. `can_upgrade` is `None` when it reported no version.
pub struct Launcher<'a> {
    pub path: &'a Path,
    pub can_upgrade: Option<bool>,
    pub protected: bool,
    pub stderr: Option<&'a str>,
}

/// Repair a broken managed install with its installer, not a second npm copy.
pub fn install_action(launcher: Option<Launcher<'_>>) -> Result<InstallAction, String> {
    match launcher {
        Some(launcher) if launcher.can_upgrade.is_none() && launcher.protected => Err(format!(
            "{}\nNo second gentle-pi copy was installed.",
            failed_version(launcher.path, launcher.stderr)
        )),
        Some(launcher) if launcher.can_upgrade == Some(true) => Ok(InstallAction::Upgrade),
        _ => Ok(InstallAction::Npm),
    }
}

/// The first meaningful stderr line, bounded for the setup status.
pub fn failed_version(path: &Path, stderr: Option<&str>) -> String {
    // The error first: a long path must not push it out of a truncated line.
    match stderr.and_then(|text| text.lines().map(str::trim).find(|line| !line.is_empty())) {
        Some(line) => format!(
            "{} ({})",
            crate::text::first_line(line, 300),
            path.display()
        ),
        None => format!(
            "{} didn't report its version (Node.js or Pi may be missing)",
            path.display()
        ),
    }
}

/// The provider check from Gentle Shell's own model registry (the Desktop
/// `get_auth_status`): the providers it has usable credentials for, by id,
/// whatever their source (a sign-in, an environment variable, `models.json`,
/// an extension), or why the check fails.
pub fn registry_sign_in<'a>(
    configured: impl IntoIterator<Item = &'a str>,
) -> Result<String, String> {
    let configured: Vec<&str> = configured.into_iter().collect();
    if configured.is_empty() {
        Err("No provider signed in".to_string())
    } else {
        Ok(format!("Signed in: {}", configured.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_answers_the_provider_check() {
        assert_eq!(
            registry_sign_in(["anthropic", "groq"]),
            Ok("Signed in: anthropic, groq".to_string())
        );
        assert_eq!(
            registry_sign_in([]),
            Err("No provider signed in".to_string())
        );
    }

    #[test]
    fn protected_launcher_without_a_version_refuses_npm() {
        let path = Path::new(r"C:\Local\gentle-shell\bin\gentle-shell.cmd");
        let stderr =
            "gentle-shell: no active Gentle Shell version; run the Gentle Shell installer again.";
        let found = |stderr| Launcher {
            path,
            can_upgrade: None,
            protected: true,
            stderr,
        };
        assert_eq!(
            install_action(Some(found(Some(stderr)))).unwrap_err(),
            format!(
                "{stderr} ({})\nNo second gentle-pi copy was installed.",
                path.display()
            )
        );
        assert_eq!(
            install_action(Some(found(None))).unwrap_err(),
            format!(
                "{} didn't report its version (Node.js or Pi may be missing)\nNo second gentle-pi copy was installed.",
                path.display()
            )
        );
    }

    #[test]
    fn failed_version_reports_bounded_meaningful_stderr_or_the_old_guess() {
        let path = Path::new(r"C:\Local\gentle-shell\bin\gentle-shell.cmd");
        let stderr =
            "gentle-shell: no active Gentle Shell version; run the Gentle Shell installer again.";
        assert_eq!(
            failed_version(path, Some(&format!("\r\n  {stderr}  \r\nextra detail\r\n"))),
            format!("{stderr} ({})", path.display())
        );
        for empty in [None, Some(""), Some(" \r\n \t\n")] {
            assert_eq!(
                failed_version(path, empty),
                format!(
                    "{} didn't report its version (Node.js or Pi may be missing)",
                    path.display()
                )
            );
        }
        let long = "é".repeat(301);
        assert_eq!(
            failed_version(path, Some(&long)),
            format!("{}… ({})", "é".repeat(300), path.display())
        );
    }

    #[test]
    fn npm_and_upgrade_choices_are_preserved() {
        let path = Path::new("/npm/gentle-shell");
        assert_eq!(install_action(None), Ok(InstallAction::Npm));
        for can_upgrade in [None, Some(false)] {
            assert_eq!(
                install_action(Some(Launcher {
                    path,
                    can_upgrade,
                    protected: false,
                    stderr: Some("node missing"),
                })),
                Ok(InstallAction::Npm)
            );
        }
        for protected in [false, true] {
            assert_eq!(
                install_action(Some(Launcher {
                    path,
                    can_upgrade: Some(true),
                    protected,
                    stderr: None,
                })),
                Ok(InstallAction::Upgrade)
            );
        }
    }
}
