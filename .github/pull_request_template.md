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

Reviewers: `/mcp` requires a bearer token and the observer uses a separate secret.
Preserve the loopback default and wildcard-bind rejection. A non-loopback bind
needs the token plus `--allow-non-loopback` and still warns that there is no TLS.
Host/Origin checks are not authentication. Do not add a no-auth mode.
