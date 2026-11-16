# tinyAgentAccess

An open-source, self-hostable, local-first Rust rebuild of the Amazon WorkSpaces for AI agents MCP server. AWS-compatible native tool names, running on your own computer, with no AWS dependency.

**Pre-alpha — milestone 2:** real desktop capture and input, with local MCP transport, sessions, schemas and config. X11 is exercised end to end; macOS, Windows and Wayland need real-session validation.

**Security: no authentication yet. Binds to localhost by default. Do not expose this server to a network or the public internet.** Security: see [SECURITY.md](SECURITY.md).

```sh
cargo run -p tinyagentaccess -- serve
# Optional: --config config.example.toml --bind 127.0.0.1:8765
cargo run -p tinyagentaccess -- print-config
```

Stateful curl smoke test (requires Python 3 to extract the session header):

```sh
curl -sS -D /tmp/taa-headers -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"curl","version":"1"}}}' \
  http://127.0.0.1:8765/mcp
TAA_SESSION=$(python3 -c 'print(next(x.split(":",1)[1].strip() for x in open("/tmp/taa-headers") if x.lower().startswith("mcp-session-id:")))')
curl -sS -H "Mcp-Session-Id: $TAA_SESSION" -H 'MCP-Protocol-Version: 2025-11-25' \
  -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","method":"notifications/initialized"}' http://127.0.0.1:8765/mcp
curl -sS -H "Mcp-Session-Id: $TAA_SESSION" -H 'MCP-Protocol-Version: 2025-11-25' \
  -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' http://127.0.0.1:8765/mcp
curl -sS -H "Mcp-Session-Id: $TAA_SESSION" -H 'MCP-Protocol-Version: 2025-11-25' \
  -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"agentaccess___wait","arguments":{"duration":0.001}}}' http://127.0.0.1:8765/mcp
curl -sS -i -X DELETE -H "Mcp-Session-Id: $TAA_SESSION" -H 'MCP-Protocol-Version: 2025-11-25' http://127.0.0.1:8765/mcp
```

Run SDK interop against the running server with `/tmp/mcpvenv/bin/python scripts/interop_python_sdk.py --url http://127.0.0.1:8765/mcp`, or install `mcp==2.1.1` in your own venv and run the script there. It tests both legacy/stateful and 2026-07-28/stateless modes.

Desktop requirements:

- **X11:** run inside the graphical session with `DISPLAY` and Xauthority access. Uses pure Rust x11rb, XTest input, RandR primary monitor and GetImage capture. No Xlib/libxcb development packages.
- **Wayland (experimental):** install xdg-desktop-portal plus the GNOME/KDE backend. Approve Screenshot and RemoteDesktop access, selecting your primary monitor. Persistent consent tokens are stored with mode 0600 under `$XDG_STATE_HOME/tinyagentaccess/portal-restore-token` (default `~/.local/state/...`). Capture uses the Screenshot portal, without libpipewire; use a **single-monitor session** for now because that API may return the entire desktop. Known Screenshot/RemoteDesktop ratio mismatches fail before pointer injection; equal ratios cannot prove monitor identity. The portal cannot force primary-monitor capture or reliably honor `include_cursor=false`/`true`. Portal Unicode delivery still requires compositor testing. Consent waits are bounded to 60 seconds; cached session info stays available.
- **macOS:** grant Screen Recording and Accessibility to the terminal/app launching the server, then restart it. CoreGraphics capture and CGEvent input use Retina scaling. CGDisplayCreateImage is deprecated and may need replacement on future macOS releases.
- **Windows:** GDI primary-display capture and DPI-aware SendInput. Use an unlocked interactive desktop. UAC/secure desktop and higher-integrity windows cannot be controlled. Code is cross-compiled here; GUI validation is still needed.

M2 targets the **primary display only** (Wayland currently requires one display). Default images are 1280×720, aspect-preserving with centered black bars; clicks in bars clamp to the display edge. Native mode returns physical capture dimensions. All desktop operations run in blocking workers. PNG and JPEG are supported. See [keys](docs/keys.md), [permissions](docs/permissions.md), [testing](docs/testing.md) and the [design limitations](docs/design.md).

Linux auto selection prefers Wayland whenever `WAYLAND_DISPLAY` or `XDG_SESSION_TYPE=wayland` is present, even when `DISPLAY` also exists: XWayland XTest cannot control native Wayland windows. Override with `[platform] linux_backend = "auto" | "wayland" | "x11"`. With no graphical session the server starts and desktop calls return an actionable error. No TLS or C cross-compiler is required for Windows GNU type-checks.

Workspace: `crates/tinyagentaccess` (CLI), `crates/taa-mcp` (HTTP/session lifecycle), `crates/taa-tools` (config, registry, validation, dispatch), `crates/taa-platform` (typed desktop boundary, shared images/keys, OS backends and an opt-in `test-support` recording mock).

See [verified AWS parity facts](docs/aws-parity.md), [design and compatibility decisions](docs/design.md), and [example config](config.example.toml). Validate with `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace`.

Licensed under the [MIT License](LICENSE).

Wildcard binds (`0.0.0.0` / `::`) are rejected because this server has no authentication. Use the loopback default or an explicit concrete IP; non-loopback binds emit a no-auth warning and only that IP or loopback Host values are accepted.

Local screenshot saving is optional and disabled by default:

```toml
[screenshots]
save_dir = "" # or an absolute private directory, e.g. /home/me/screenshots
```

Returned standalone and sequence terminal images are saved under `agentaccess/screenshots/year=YYYY/month=MM/day=DD/{session-id}/` using UTC epoch-millisecond filenames and the real PNG/JPEG extension. Unix directories/files use 0700/0600 when created; Windows inherits directory ACLs. Save failures warn on stderr and leave tool results intact. Relative save paths are rejected. No cloud upload occurs.

### MCP tool forwarding

Set `[agent_access] forward_mcp_tools = true` and optionally set
`[forwarding] config_path = "/absolute/path/redirection.json"`. The redirection file
must be UTF-8 without a BOM:

```json
{"mcpServers":{"example":{"command":"/absolute/path/mcp-server","args":["--stdio"]}}}
```

Only stdio is supported. Children inherit the server environment; `env`, `cwd` and
other unknown keys are ignored with warnings. Tools appear as
`forwarded___example___<tool>`. Lists and calls have a five-second timeout. Each MCP
session owns its children and closes them on session expiry or closure. Forwarding-only
operation (`computer_vision = false`, `computer_input = false`, forwarding enabled)
is supported. Empty config_path uses `/etc/dcv/mcp_server_redirection_config.json`
on Linux, `C:\ProgramData\NICE\dcv\mcp_server_redirection_config.json` on Windows,
and the provisional `/Library/Application Support/tinyagentaccess/mcp_server_redirection_config.json`
on macOS. See [design decisions](docs/design.md#mcp-tool-forwarding-m4).

Milestone 5 adds computer-use dialects selected per request with
`x-amzn-agentaccess-computer-use-interface-id`: `ANTHROPIC_COMPUTER_20250124`,
`ANTHROPIC_COMPUTER_20251124`, `ANTHROPIC_COMPUTER_TOOLSET_20260801`,
`OPENAI_COMPUTER_V1`, and `GOOGLE_COMPUTER_USE_3X_DESKTOP_V1`.
Send it on both tools/list and every tools/call. Omitted/empty selects native tools;
unknown values return JSON-RPC -32602. Dialects are **best-effort, unverified vs AWS**;
all schemas are **PROVISIONAL vendor-derived, not AWS-confirmed**. Only standard
desktop tools are replaced; session, wait, sequence and forwarded tools stay available.
See the [local conversion choices](docs/design.md#m5-computer-use-dialect-adapters).
