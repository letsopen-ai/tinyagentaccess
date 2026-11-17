## Problem

What concrete problem does this address?

## Result

Describe the resulting behavior and relevant tests or documentation.

## Limits

List remaining gaps, platform validation limits, and PROVISIONAL / TINY-PROPOSAL choices.

## Validation

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] Or, docs-only: `git diff --check` and factual / link review
- [ ] Relevant tests and docs updated for behavior changes
- [ ] No secrets, personal paths, or sensitive screenshots/replays

Reviewers: this server has **no authentication**. Preserve loopback defaults,
wildcard-bind rejection, and the warning on non-loopback binds.
