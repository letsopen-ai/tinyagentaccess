# Engineering guidelines

- Use feature branches and pull requests targeting `main`; no direct commits or force-pushes to `main`.
- Never invent AWS API facts. Consult [AWS parity research](docs/aws-parity.md) and [design decisions](docs/design.md); cite evidence and mark undocumented local choices `PROVISIONAL` or `TINY-PROPOSAL`.
- Do not edit `docs/aws-parity.md` unless the maintainer explicitly authorizes it.
- Authentication is required: `/mcp` needs a bearer token from a 0600 token file, and the observer needs its own secret. Do not add a no-auth mode, env bypass, or token logging, and do not put the token in the server environment. Preserve the loopback default and wildcard-bind rejection; a non-loopback bind needs the token plus explicit opt-in (`--allow-non-loopback`) **and still prints the no-TLS warning**. Host/Origin checks are not authentication.
- Keep `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` clean. Preserve Windows GNU and macOS ARM64 checks. Use Rust 1.88+ and edition 2021.
- Include relevant tests and documentation for behavior changes. Distinguish compilation, mocks, and real desktop validation.
- Never commit secrets, credentials, local configuration, tool state, personal paths, or sensitive screenshots/replays.
- Do not rewrite git history. Normalize historical authors through `.mailmap`.
