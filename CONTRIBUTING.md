# Contributing

Many people work on this repository at once. These rules keep two people from doing the same issue and make it clear which pull requests are ready. Read [`AGENTS.md`](AGENTS.md) before your first PR: it says where code goes and the design rules.

## Before you start an issue

- **Check it isn't assigned.** Someone under "Assignees" is already working on it.
- **Comment that you're taking it** ("I'll take this") and wait for a maintainer to assign you. From then on it's yours.
- **No progress in 14 days** (no PR, no comment) frees the issue for someone else. Stuck or out of time? Say so in the issue and we'll free it.
- **Something without an issue?** Open one first and describe what you propose, especially for a new feature or a design change, so we agree on it before you spend time.

## Opening your pull request

- **Open it as a Draft** while you're still working. Draft means "in progress, don't review yet".
- **Link the issue** with `Closes #123` in the description: the issue shows there's a PR, and it closes when the PR merges.
- **Mark it "Ready for review"** only when:
  - [ ] `cargo fmt` is applied.
  - [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings` passes.
  - [ ] `cargo test` passes.
  - [ ] You tried it for real, in an isolated copy ([`docs/development.md`](docs/development.md#isolated-testing)).
  - [ ] Visual changes have screenshots in the PR.
  - [ ] The PR says which systems you tried it on (macOS, Windows, Linux).

## What a PR's state means

|State|Meaning|
|---|---|
|Draft|In progress; not reviewed yet.|
|Ready for review|Ready for a maintainer to review.|
|Changes requested|There are comments to address. Reply in the PR when they're done so we look again.|
|Needs testing on Windows / Linux|The code is fine; someone with that system has to try it. If you have it, your help counts.|
|Approved and CI green|It gets merged into `main`.|

## Project rules

- **One PR, one topic.** A bug found along the way goes in its own PR ([ADR 0003](docs/adr/0003-behavior-preserving-refactors.md)).
- **Changes to `.github/`, `Cargo.*`, `build.rs`, `packaging/` or the updater** need a Code Owner's review ([`docs/security.md`](docs/security.md)).
- **Never test on the app you're using:** use an isolated copy so your data stays safe ([`AGENTS.md`](AGENTS.md#testing-without-breaking-anything)).
