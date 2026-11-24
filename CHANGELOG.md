# Changelog

All notable changes are recorded here in [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) style.

## [Unreleased]

### Added

- Bearer-token authentication (**TINY-PROPOSAL**, not AWS SigV4/IAM or MCP OAuth): every `/mcp` request needs `Authorization: Bearer <token>` from a 0600 / owner-only token file (default created on first `serve`). Missing, wrong, or duplicate headers return 401 without `WWW-Authenticate` or `resource_metadata`, before the body is read.
- A separate observer secret (`observe-token`) for `/observe/sessions`, `/observe/latest/{id}` and Stop, delivered in the `/observe#<secret>` URL fragment and sent as `X-TAA-Observe-Token`; observer responses send `Referrer-Policy: no-referrer`.
- `serve --token-file` / `--allow-non-loopback`, `[auth]` config (`token_file`, `allow_non_loopback`), `tinyagentaccess rotate-token`, and `tinyagentaccess observe-url`.
- `doctor` reports the auth token path, existence, and permission status (text and an additive JSON `auth` object); it never prints or creates a token.

### Changed

- **BREAKING:** clients must send the bearer token. Existing unauthenticated OpenCode, curl, and other configs get 401 until updated (README has OpenCode, Claude Code, and Codex snippets; header setups were not tested against live clients).
- **BREAKING:** a named `--config` path that is missing or unreadable now fails startup instead of silently using defaults.
- **BREAKING:** a concrete non-loopback bind is rejected unless `--allow-non-loopback` (or `[auth] allow_non_loopback = true`) is set; it still warns that there is no TLS. Wildcard binds remain rejected.
- Startup now prints the token file path, never the token.
- `serve` and `observe-url` reject an agent token and observer secret that share a path, a link, or a value. First-time token creation publishes a complete file atomically. `TAA_TOKEN` is removed from forwarded MCP server environments. On Windows a token file must be owned by the current user. For non-loopback binds the observer URL printed is the loopback one, with an SSH-forward note. Interop scripts send the bearer token (`TAA_TOKEN` or `--token-file`).
- Documentation: README, SECURITY.md, AGENTS.md, `docs/design.md`, and `config.example.toml` describe authentication. `docs/aws-parity.md` §12 now describes the local token-file departure and retains the AWS SigV4, IAM, and OAuth 2.1 via AWS Sign-In evidence.

## [0.1.2] - 2026-11-24

### Added

- Read-only `tinyagentaccess doctor [--json]` command that reports the version, binary path and SHA-256, parent process chain and likely host app, SSH / non-GUI hints, a remedy, and on macOS the no-prompt Screen Recording and Accessibility preflight results and ScreenCaptureKit availability. Exit codes: 0 pass or not applicable, 1 macOS preflight failed, 2 usage or doctor error ([#14](https://github.com/letsopen-ai/tinyagentaccess/pull/14)).
- Documentation: macOS first-run guidance (host-app permissions, non-notarized build, restart after granting), an OpenCode loopback config for `http://127.0.0.1:8765/mcp`, and a README known-issue note about the 10 s ScreenCaptureKit timeout when two servers capture (see Changed) ([#14](https://github.com/letsopen-ai/tinyagentaccess/pull/14)).

### Changed

- Documentation: the two-capturing-servers ScreenCaptureKit timeout is now confirmed by a real-Mac isolation run (2026-11-24). Run at most one capturing server per desktop session. In that run, a second process that attempted capture after another had already captured timed out after 10 s and left both processes unable to capture until restarted. Documented as a deployment limitation, not a proven code bug; no code change ([#16](https://github.com/letsopen-ai/tinyagentaccess/pull/16)).

### Fixed

- `doctor` now recognizes both `Grok Bot` and `Grok Bot Helper` as macOS permission hosts instead of reporting an unknown host. If the nearest GUI match would be `Grok Bot Helper` and `Grok Bot` is also in the chain, the app (`Grok Bot`) is preferred; a helper-only chain still reports `Grok Bot Helper`. The same helper-to-app preference applies to other known hosts (for example `Code Helper` and `Code`). A nearer known terminal is still preferred over a GUI host ([#15](https://github.com/letsopen-ai/tinyagentaccess/pull/15)).

## [0.1.1] - 2026-11-18

### Fixed

- macOS scroll reported success but missed the view under the requested coordinates: the wheel event inherited the sampled cursor location instead of `self.position`. The constructor now calls `CGEventSetLocation` ([#12](https://github.com/letsopen-ai/tinyagentaccess/pull/12)).
- The macOS Rust FFI declared `wheel1` as variadic, contrary to `CGEvent.h` (`int32_t wheel1, ...`), producing a garbage one-notch `NSEvent.scrollingDeltaY` of `-24760` on Apple ARM64. It now uses non-variadic `CGEventCreateScrollWheelEvent2` ([#12](https://github.com/letsopen-ai/tinyagentaccess/pull/12)).

### Changed

- Moved macOS CI to standard GitHub-hosted runners ([#10](https://github.com/letsopen-ai/tinyagentaccess/pull/10)).
- Clarified that observer Stop ends one MCP session and is not a lockout ([#11](https://github.com/letsopen-ai/tinyagentaccess/pull/11)).

## [0.1.0] - 2026-11-17

### Added

- MCP HTTP transport and session lifecycle ([#1](https://github.com/letsopen-ai/tinyagentaccess/pull/1)).
- Desktop capture and input backends for X11, experimental Wayland, macOS, and Windows ([#2](https://github.com/letsopen-ai/tinyagentaccess/pull/2)).
- Tool sequences and session cleanup ([#3](https://github.com/letsopen-ai/tinyagentaccess/pull/3)).
- Local stdio MCP tool forwarding ([#4](https://github.com/letsopen-ai/tinyagentaccess/pull/4)).
- Best-effort, PROVISIONAL computer-use dialect adapters ([#5](https://github.com/letsopen-ai/tinyagentaccess/pull/5)).
- TINY-PROPOSAL local replay and observer with human Stop ([#6](https://github.com/letsopen-ai/tinyagentaccess/pull/6)).
- macOS ScreenCaptureKit screenshots with legacy capture fallback ([#7](https://github.com/letsopen-ai/tinyagentaccess/pull/7)).
- Windows x86_64 MSVC `.zip` release archive.
- Full third-party license bundle (`THIRD_PARTY_LICENSES.html`) generated by cargo-about, shipped in archives and as a signed standalone asset.
- Release archives, checksums, and license bundle signed with keyless cosign (Sigstore bundles).

### Changed

- Prepared community documentation, crate metadata, dependency license checks, and CI/release packaging.

[Unreleased]: https://github.com/letsopen-ai/tinyagentaccess/compare/v0.1.2...HEAD

[0.1.2]: https://github.com/letsopen-ai/tinyagentaccess/releases/tag/v0.1.2
[0.1.1]: https://github.com/letsopen-ai/tinyagentaccess/releases/tag/v0.1.1
[0.1.0]: https://github.com/letsopen-ai/tinyagentaccess/releases/tag/v0.1.0
