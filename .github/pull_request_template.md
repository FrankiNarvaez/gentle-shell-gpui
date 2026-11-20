## What and why

<!-- What changes for the user, and why. `Closes #123` if there is an issue. Open as a Draft until the checklist is done (CONTRIBUTING.md). -->

## How it was tested

<!-- Commands, systems you tried it on (macOS, Windows, Linux), an isolated-copy smoke test (docs/development.md), screenshots for visual changes. -->

## Security

Mark what this PR does; a maintainer reads those parts line by line (docs/security.md).

- [ ] Adds or bumps dependencies (`Cargo.toml` / `Cargo.lock`): which and why:
- [ ] Starts processes, opens network connections or downloads files
- [ ] Reads or writes files outside the app's data dir (configs, credentials, home)
- [ ] Touches CI (`.github/`), `build.rs`, `packaging/` or the self-updater
- [ ] Adds binary files (images, fonts, archives): where they come from:
- [ ] None of the above

## Checklist

- [ ] `cargo fmt`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`
- [ ] One change per PR; refactors separate and marked `(no behavior change)`
