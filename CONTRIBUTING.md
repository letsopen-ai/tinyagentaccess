# Contributing

Welcome! Small fixes, careful bug reports, and focused improvements are welcome.
Please follow our [Code of Conduct](CODE_OF_CONDUCT.md) and report vulnerabilities
privately using [SECURITY.md](SECURITY.md). Maintainer: [@niechen](https://github.com/niechen).

## Development

Fork the repository, clone your fork, and create a feature branch:

```sh
git clone https://github.com/YOUR-USERNAME/tinyagentaccess.git
cd tinyagentaccess
git switch -c fix/your-change
```

Use Rust 1.88 or newer (edition 2021). Before submitting:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Run `cargo fmt --all` to format changes. The local dependency license gate is
`cargo deny check licenses` with [deny.toml](deny.toml).
Documentation-only changes may use `git diff --check` plus factual and link review.

This server has **no authentication**. Testers must bind loopback and keep screen
captures, replay data, credentials, and personal configuration out of contributions.
X11 needs access to the graphical session; Wayland needs portal consent and remains
experimental. macOS needs Screen Recording and Accessibility permissions. Windows
needs an unlocked interactive desktop. See [permissions](docs/permissions.md) and
[testing](docs/testing.md) for platform setup, checks, and validation limits.

## Pull requests

Target `main` from a feature branch. Do not commit directly or force-push to `main`.
Keep one problem per PR, use the [PR template](.github/pull_request_template.md),
and include tests and documentation for behavior changes. Separate verified AWS
facts from local choices; see [engineering guidelines](AGENTS.md).

PR checks use standard GitHub-hosted Ubuntu, macOS, and Windows runners. All CI
jobs run on fork PRs, including Ubuntu tests with the Xvfb X11 end-to-end test,
cross-platform checks, and license checks. After publication, maintainers should require
approval for external contributors' fork workflow runs. Approved runs use the
fork's workflow files, so human review still matters.

Release packaging is separate from PR CI. Pushing a `v*` tag runs `release.yml`,
which builds Linux musl, macOS, and Windows archives containing
`THIRD_PARTY_LICENSES.html`, plus checksums and keyless cosign signatures, into a
draft GitHub release that a maintainer publishes. Hosted macOS release jobs run
only when the repository is public; manual runs upload artifacts without creating
a GitHub release. Publishing to crates.io is not enabled.
