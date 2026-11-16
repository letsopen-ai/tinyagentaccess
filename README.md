# tinyAgentAccess

An open-source, self-hostable, local-first Rust rebuild of the Amazon WorkSpaces for AI agents MCP server. AWS-compatible native tool names, running on your own computer, with no AWS dependency.

**Pre-alpha — milestone 1:** MCP transport, sessions, schemas, config, and session tools work. Desktop capture and input are not available yet.

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

macOS, Linux, and Windows build the scaffold; real desktop backends arrive later. No TLS or C cross-compiler is required for the Windows GNU type-check.

Workspace: `crates/tinyagentaccess` (CLI), `crates/taa-mcp` (HTTP/session lifecycle), `crates/taa-tools` (config, registry, validation, dispatch), `crates/taa-platform` (desktop trait, unavailable/recording mock).

See [verified AWS parity facts](docs/aws-parity.md), [design and compatibility decisions](docs/design.md), and [example config](config.example.toml). Validate with `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace`.

Licensed under the [MIT License](LICENSE).

Wildcard binds (`0.0.0.0` / `::`) are rejected because this server has no authentication. Use the loopback default or an explicit concrete IP; non-loopback binds emit a no-auth warning and only that IP or loopback Host values are accepted.
