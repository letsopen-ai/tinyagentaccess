# tinyAgentAccess design (milestones 1–7)

This is a local implementation, with no AWS SDK, credentials, cloud connection, TLS stack, or authentication. The evidence source is [aws-parity.md](aws-parity.md); this document records implementation choices, not additional claims about AWS.

## Architecture and request flow

`tinyagentaccess` reads TOML and CLI overrides, validates configuration, binds a Tokio listener, and serves the `taa-mcp` Axum router at `/mcp`. `taa-mcp` checks Host/Origin, parses JSON-RPC, chooses protocol/session, checks the per-request interface, and calls `taa-tools`. `taa-tools` owns native schemas, gating, validation, session-independent waits, and sequential desktop dispatch. `taa-platform` provides an object-safe `Platform` boundary for screenshots, screen size, movement/click count/button/modifiers, button down/up, drag, directional wheel ticks/modifiers, key combinations, holds, and text.

The executable selects a target-gated real backend. `Platform` accepts typed `Operation` values with physical primary-display coordinates, `Button`, `KeyCombo` and `ScrollDirection`. It returns a tightly packed RGBA `Frame` with physical and logical sizes and optional cursor metadata. JSON parsing, schemas and key/modifier validation live in `taa-tools`; the portable key parser lives in `taa-platform::keys`. `capture_geometry` refreshes dimensions without image transfer where supported. `screen_size` is cached on Linux. `MockPlatform` is enabled only for tests or the explicit `test-support` feature; it records typed operations and supplies a synthetic frame.

`taa-tools::execute` moves all native operations, image processing and encoding to `tokio::task::spawn_blocking`, retaining global async dispatch serialization. Backends also serialize native connection access. Wait uses Tokio; hold uses the blocking worker. The portal backend owns a one-worker Tokio runtime kept alive between calls, and attempts bounded session closure before shutting the runtime down in the background on drop. This keeps portal activity off the main runtime. Backend initialization is lazy except the inexpensive initial X11 connection/geometry lookup on the startup worker; absence of a display does not prevent server startup.

## Transport and sessions

Stateful versions are 2025-03-26, 2025-06-18, and 2025-11-25. `initialize` echoes a supported request or selects 2025-11-25, returns tools-only capabilities (`listChanged: false`), serverInfo `tinyAgentAccess` and the Cargo version, plus a random UUID `Mcp-Session-Id`. Later POSTs require it, including initialized notifications (202 empty) and ping. DELETE returns 204; unknown/expired/closed IDs return HTTP 404. The protocol header, when supplied, must match the session. Unsupported POST protocol headers return structured HTTP 400 / JSON-RPC -32022, regardless of method. DELETE retains the legacy transport-only HTTP 400 for unsupported headers.

The installed official Python MCP SDK 2.1.1 confirms the 2026-07-28 wire protocol: `server/discover`, no initialize, and no transport session ID. Discovery returns `supportedVersions`, capabilities, `resultType: complete`, `ttlMs: 0`, `cacheScope: private`, and display-only `_meta["io.modelcontextprotocol/serverInfo"]`. Each modern request carries `_meta["io.modelcontextprotocol/protocolVersion"]`, `_meta["io.modelcontextprotocol/clientCapabilities"]`, optional clientInfo, and HTTP `MCP-Protocol-Version`, `Mcp-Method`, and (tools/call) `Mcp-Name`. Header mismatch is -32020. Modern list results include required cache fields; tool results include `resultType: complete`. Plain `application/json` responses are accepted by the SDK; no SSE is needed. GET and modern DELETE return 405.

Unsupported POST versions (including discovery) receive JSON-RPC -32022 (`UNSUPPORTED_PROTOCOL_VERSION`), with `data.supported` and `data.requested`, matching SDK negotiation. This code is not used as a generic AWS error. Discovery and tool listing do not allocate desktop sessions.

Stateless calls require `arguments.mcpSessionId`. NEW creates a desktop session; later calls reuse its UUID. Every modern tool result includes both `result.mcpSessionId` and a JSON text block containing the ID, including failures and close. JSON-RPC errors associated with a supplied existing ID include `error.data.mcpSessionId`. HTTP/security errors and malformed requests are transport failures, not tool results.

[TINY-PROPOSAL] Both kinds share `max_sessions` (default 1). Refusal uses `CONNECTION_LIMIT_REACHED: Another agentic client is already connected`: stateful initialization returns JSON-RPC -32000, stateless NEW returns a CLIENT_ERROR tool result. Unknown stateless IDs return `CLIENT_ERROR: Unknown mcpSessionId`. Missing ID returns `CLIENT_ERROR: mcpSessionId is required` with an empty ID marker; refused NEW retains NEW because no ID was allocated. Close returns `Session closed`, including repeat close for a remembered ID. Closed IDs are remembered for a 300-second retry window, then lazily pruned on session access/allocation. Idle timeout defaults to 780 seconds, pruned lazily on session access/allocation; expired IDs are unknown, not automatically reconnected. A shared async mutex serializes desktop tool dispatch and waits across sessions. Cached `get_session_info` bypasses that mutex so pending desktop consent cannot block metadata; it still checks and pins the session atomically. Session existence is checked again after acquiring it, so queued work cannot execute after closure. Admitted work is pinned against idle expiry until completion or cancellation; completion restarts its idle timer. Closure does not interrupt work already admitted. Queued work may expire before admission. This is deliberately simpler than AWS reconnect semantics.

## Security

Default bind is `127.0.0.1:8765`. `--bind` overrides TOML. Wildcard binds (`0.0.0.0` and `::`) fail configuration validation with a no-auth safety error; use a concrete IP socket address. A concrete non-loopback bind is allowed and emits a loud no-auth warning. Host must name localhost, a loopback IP, or the explicitly bound non-loopback IP. Arbitrary DNS names are rejected to reduce DNS rebinding exposure. Origin, if present, must use http/https and a loopback IP or localhost authority; other origins, null, userinfo, paths, and duplicate Host/Origin headers return 403. POST requires Content-Type application/json (parameters allowed), returning 415 otherwise. This JSON-response-only server requires Accept application/json (application/* and */* allowed), returning 406 otherwise; SSE acceptance is not required, matching the SDK JSON-response mode. Duplicate or non-UTF-8 routing headers and duplicate Content-Type return 400. Host/Origin remain 403 (an intentional difference from the SDK’s 421 for bad Host). Unknown AWS headers and query parameters are ignored. These checks do not authenticate local clients. Never expose this server.

## Registry, validation, outputs and errors

[TINY-PROPOSAL] All schemas are provisional, marked at the schema generator, and are not AWS-verified. Objects reject extra argument keys, coordinates are signed 64-bit JSON integer representations in [0,width) / [0,height) (2.0 is deliberately rejected; AWS integral-float acceptance remains unknown), scroll_amount is a signed integer in [-12,000,12,000] (sign semantics retained from M1, bounded for local safety); M2 validates the portable key vocabulary described in keys.md. Unicode scalar values count toward text limits. InputSchema publishes documented duration, text, direction, and action-count limits. The execute_sequence maxItems constraint is enforced by its bespoke preflight to retain sequence-summary rejection diagnostics; the generic property validator is deliberately not a full JSON Schema engine. Modern schemas require mcpSessionId. Names use agentaccess___; short names are accepted only within sequences.

[TINY-PROPOSAL] Gated tools are hidden and calls return `CLIENT_ERROR: Unknown tool` with `isError: true`, as do unknown names. Vision gates screenshot; input gates mouse/keyboard and execute_sequence. wait/get_session_info always exist; close_session is listed only in stateless mode. Input without vision is a config error.

[TINY-PROPOSAL] Missing stateful header is HTTP 400 plain UTF-8 text exactly `Missing Mcp-Session-Id header`. JSON-RPC parse/invalid-request/method/params errors use -32700/-32600/-32601/-32602; internal/interface-not-implemented errors use -32603. Modern JSON-RPC errors map parse/invalid-request/invalid-params/header-mismatch/unsupported-version to HTTP 400 and method-not-found to 404; internal errors retain HTTP 200. Legacy parse/envelope errors return HTTP 400; handler method/parameter errors remain JSON-RPC responses with HTTP 200. Invalid request IDs return null, never echo untrusted arrays/objects. Params must be an object. Valid modern notifications are acknowledged and dropped with 202 using only the version header, without requiring request metadata/routing headers, matching the SDK.

POST bodies have an explicit 2 MiB cap (413), preserving Axum’s existing Bytes default. serde_json’s default 128 nesting limit remains enabled; excessive nesting produces a parse error, not recursive dispatch. Batches are accepted within established 2025-03-26 sessions, excluding initialize, and notification members produce no result. Nested arrays and other non-object members receive -32600 member errors; dispatch never recursively invokes the HTTP handler. Other versions and empty arrays reject batches with -32600. Receiving batches is required by the [2025-03-26 base protocol](https://modelcontextprotocol.io/specification/2025-03-26/basic#batching); later versions removed batching. Native interface is omitted/empty. The five known case-sensitive interface IDs are represented by an enum and return -32603 with `Computer-use interface is not implemented yet`; other values return -32602. No silent dialect fallback.

[TINY-PROPOSAL] wait sleeps using Tokio and succeeds with `Wait completed`; invalid duration returns the CLIENT_ERROR text `wait duration must be a number between 0.001 and 5.0 seconds`. Desktop successes use `Action completed`. Tool failures use isError and CLIENT_ERROR / SERVER_ERROR prefixes. Screenshot output uses an MCP image block with base64 data and platform MIME type; PNG/JPEG encoding and scaling are shared, described below.

## M3 sequences, closure and local screenshots

`execute_sequence` preflights every action before any desktop dispatch. It accepts listed native names as sibling-key objects, returns short names in diagnostics, rejects nested screenshot/sequence, hidden/unknown tools, session tools and forwarded/interface tools. Count is checked first, then top-level arguments and ordered per-action standalone validators, then cumulative wait and text totals. Wait totals allow a 1 ns tolerance for floating-point summation at the inclusive 10-second boundary but display rounded to one decimal; text counts Unicode scalar values, matching standalone type_text. Rejection is one JSON text block, with the sample's exact prefixed stop_reason, guidance, and skipped/reason rows. Stateless transport appends its usual ID block.

The desktop lock covers the entire run. Each native action stays on a blocking worker. Execution stops at its first failure. A 60-second deadline is checked before every action and before terminal capture. Waits and stabilization clamp to the remaining budget; waits interrupted by expiry count as executed errors, with later actions skipped. PROVISIONAL (AWS documents only the text `Deadline reached before action N`): an interrupted wait at index i reports action i+1 as the first undispatched action, while stopped_at_index remains i, the wait where execution stopped. Blocking native actions cannot safely be interrupted; after they return the next admission check observes expiry. A capture completing after deadline is discarded (text-only deadline result). `SequenceLimits` exposes deadline and stabilization for deterministic tests.

Success includes per-action duration_ms and summary counts, a 300 ms stabilization pause unless the last action is wait, and terminal screenshot timing plus image using the standalone encoder. Failed capture yields terminal_screenshot status error, isError and text only. Vision disabled skips terminal capture successfully; execute_sequence itself retains M1 input gating, so normal transport configurations cannot expose it with vision disabled. Both disabled leaves wait and metadata (plus modern close) visible.

One server-wide record tracks the persistent left button, its owning session and the current pointer position, including inside sequences. Button operations and drag always use the left button. Every successful pointer-positioning operation updates the held position; left click, button-up and drag clear ownership across all sessions. Each press takes ownership. Close, idle expiry and stateful DELETE release only a button still owned by that session, on a blocking worker serialized with desktop dispatch. Release uses the last known current position because the platform Button API requires a point. Tracking and dispatch also share a mutex so cancellation cannot interleave their updates. Key holds already release synchronously in each backend. Stateful DELETE also serializes release with desktop work. expire_streaming_session defaults false and has no local effect. Repeated modern close returns `Session closed`; later desktop calls fail.

Every image actually returned through HTTP, standalone or sequence terminal, can be saved through `[screenshots] save_dir`. Empty disables saving by default; a nonempty path must be absolute (relative paths are rejected at configuration validation). The sink runs on a blocking worker and logs failures to stderr without changing the tool response. Layout is `agentaccess/screenshots/year=YYYY/month=MM/day=DD/{mcp-session-id}/{epoch-millis}-{counter}.{png|jpg}` in UTC. The ID accepts only ASCII alphanumerics and hyphens. New directories use 0700 and files 0600 on Unix; existing directory permissions are retained. Directory symlinks are refused; final files use create_new (atomic exclusive creation, never following an existing symlink), with counters on collisions. Windows permissions are best effort via inherited ACLs. No cloud dependency or upload exists. No new runtime crate was added; Gregorian UTC date conversion uses SystemTime and integer arithmetic. `tempfile` is test-only.

Provisional items (not published by AWS): short names are SAMPLE-only; empty/malformed rejection wording, nested/session/unknown rejection wording, top-level/per-action validation precedence, whole-sequence stopped index (20 for overflow, first crossing for cumulative totals), Unicode scalar interpretation, 300 ms pause, failure/error/skipped status and reason texts, screenshot retry guidance, interrupted-wait accounting, capture failure behavior and vision-disabled skip, standalone wait `Wait completed`, local metadata/JSON encoding, close `Session closed` and input release policy, absolute save paths, real JPEG `.jpg` extension, epoch-millisecond/counter filenames, local permission/storage policy. Code marks these with `PROVISIONAL (not published by AWS)` comments.

## Session-info mapping

[TINY-PROPOSAL] get_session_info returns JSON text, containing mcpSessionId, hostname (HOSTNAME/COMPUTERNAME, falling back to localhost), os (Rust target OS), platformBackend, screen {width,height}, protocolVersion, and serverVersion. Agent-visible size is 1280×720 by default, or platform dimensions for native. AWS stack, userId, workSpacesSessionId, fleet and imageBuilder are omitted because there is no AWS target. No streaming URL or cloud metadata is returned.

## Dropped AWS concepts

The local desktop is immediately connected (BLOCKING behaviour), so POLLING, x-amzn-agentaccess-connect-mode, and connection_status have no implementation. Stack, fleet, image-builder name, user-id, local-user, and streaming-session-url headers are ignored. Query login_hint, workspacesApplicationsStack, workspacesApplicationsUserId (and UserId aliases) are ignored. SAML/stack metadata keys aws.agentaccess/workspacesApplicationsSamlAssertion, aws.agentaccess/workspacesApplicationsStackArn, proxy saml_response/stack_arn, and aws.agentaccess/streamingSessionUrl are ignored. There is no AWS targeting, identity, OAuth/SigV4, SAML, DCV, instance expiry, S3, KMS, CloudWatch or CloudTrail dependency. launch_application, toggle_app_switcher, switch_user, and get_current_user are absent because their fleet/image-builder semantics do not apply locally.

x-amzn-agentaccess-expire-streaming-session-on-delete and close_session.expire_streaming_session are accepted no-ops: closing only releases a local MCP session. No local OS logout, application exit or process termination occurs.

## Configuration mapping

All TOML structs deny unknown keys. Missing file/sections use defaults. Invalid enums, zero session limits and input-without-vision fail startup. CLI bind overrides file bind. [TINY-PROPOSAL] defaults are local choices; AWS does not document defaults for all corresponding optional fields.

| Local key | AWS equivalent / decision |
|---|---|
| agent_access.computer_vision = true | Settings COMPUTER_VISION ENABLED |
| agent_access.computer_input = true | Settings COMPUTER_INPUT ENABLED, requires vision |
| agent_access.forward_mcp_tools = false | Settings FORWARD_MCP_TOOLS DISABLED by default; true enables per-session stdio forwarding |
| agent_access.screen_resolution = "1280x720" | ScreenResolution W_1280xH_720; local extension "native" |
| agent_access.screen_image_format = "png" | ScreenImageFormat PNG / JPEG, lowercase TOML png/jpeg |
| agent_access.user_control_mode = "disabled" | UserControlMode DISABLED / VIEW_ONLY / VIEW_STOP mapped to disabled/view_only/view_stop; local /observe UI (M7) |
| replay.enabled = false / replay.save_dir = "" | TINY-PROPOSAL local ReplaySessionEnabled substitute, required absolute path when enabled; no KmsKeyArn |
| forwarding.config_path = "" | local absolute override; empty uses OS default (see M4) |
| screenshots.save_dir = "" | local screenshot saving (M7 feature pulled into M3); absolute path or empty; never S3 |
| bind / sessions.max_sessions / sessions.idle_timeout_secs | local transport/session settings; 780-second default inspired by image-builder timeout |

At least one of vision, input or forwarding must be enabled; forwarding-only is allowed. Replay is local only; KmsKeyArn is excluded.

## M2 desktop decisions

Every decision here is a local [TINY-PROPOSAL], not verified AWS behavior.

- **Agent space:** default 1280×720, aspect-preserving centered letterbox. Fit scale is `min(1280/physical_width,720/physical_height)`. Viewport dimensions round to nearest integer; odd extra pixels go on the right/bottom. Black bars are opaque. Pointer positions outside the viewport clamp to the nearest display edge, after checking agent-space bounds.
- **Inverse:** both resampling and pointer mapping align viewport endpoints: `(agent-offset)*(physical-1)/(viewport-1)`. Input rounds to nearest integer (ties away from zero). Forward mapping uses the inverse ratio and the same rounding. For downscaling, agent→physical→agent is exact; native→agent→native necessarily loses detail. Upscaling can also quantize several agent pixels to one source pixel. The viewport is the same in both directions, avoiding independent float-scale drift. Native mode skips resize and maps physical pixels directly.
- **HiDPI:** macOS queries current CGDisplayCopyDisplayMode pixel width/height, verifies capture dimensions against that mode, and converts physical pixels to CGEvent points independently on each axis with the current primary origin on every motion; portal motion converts screenshot pixels to stream logical units; Windows sets PER_MONITOR_AWARE_V2 and converts physical coordinates to 0…65535 across the virtual desktop; X11 adds the primary monitor's root offset. Unit tests exercise 2×, 1.25× and 1.5× conversion.
- **Images:** in-crate bilinear resampling with endpoint alignment. `png` encodes RGBA and decodes portal images; `jpeg-encoder` encodes RGB at quality 85. No full image framework. Image MIME types are image/png and image/jpeg. Native frames avoid resampling.
- **Cursor:** capture APIs are asked for cursor-free pixels where possible. X11 uses XFixes ARGB cursor data, unpremultiplied and alpha blended before resize. Other backends use a small black/white arrow if a position is available (macOS event location; Windows GetCursorInfo). Cursor drawing clips at all image edges. `include_cursor=false` does not composite. Screenshot portal has no cursor exclusion option. On Wayland, `include_cursor=false` cannot guarantee cursor-free pixels, and `true` cannot guarantee cursor inclusion: the compositor controls the captured cursor. No stale synthetic arrow is overlaid on portal images.
- **Keys:** shared case-insensitive named vocabulary plus case-preserving printable characters; see keys.md. Combo presses occur in order and releases in reverse, including on returned errors. Drag uses twelve interpolated moves, 5 ms apart. Clicks pause 60 ms between clicks; macOS sets click state to 1/2/3. Modifiers apply only during the requested event. Held mouse state persists on macOS. Key input errors identify unavailable native mappings.
- **Text:** 5 ms between Unicode scalars. X11 looks up current keysyms, temporarily remaps the highest keycode for absent characters and restores its complete row, including after a returned event error. A server round trip followed by a 50 ms pause before restoration gives consumers time to resolve KeyPress; this cannot guarantee resolution by arbitrarily delayed clients. This transient remap is global to the X server; concurrent human typing can race it. macOS sends one Unicode scalar (one/two UTF-16 units) per CGEvent; Windows uses KEYEVENTF_UNICODE including surrogate pairs; portals use NotifyKeyboardKeysym. CRLF is normalized to LF, lone CR also becomes LF, and newline/tab use Return/Tab. All other Unicode control characters (C0, DEL and C1) are rejected before dispatch as CLIENT_ERROR; other Unicode scalars remain supported. No clipboard is touched. Portal support for Unicode keysyms absent from the compositor keymap remains unverified.
- **Scroll:** preserve M1 signed integer acceptance instead of introducing the brief's optional positive-only rule. Zero does nothing; negative reverses direction. For discrete APIs, nearest notch with a minimum of one for any nonzero magnitude: `max(1,(abs(ticks)+60)/120)`; no residual across calls. Magnitudes above 12,000 ticks (100 notches), including i64::MIN, return CLIENT_ERROR before dispatch; this bounds X11 round trips. Windows passes signed ticks; macOS uses line units (one line per notch, potentially smaller than a hardware wheel notch). Discrete rounding tests cover 1,60,120,179,180,360.
- **Backend selection:** auto prefers Wayland for WAYLAND_DISPLAY or XDG_SESSION_TYPE=wayland; DISPLAY alone selects X11. Never silently use XWayland for Wayland control. Explicit platform.linux_backend accepts only auto/wayland/x11; the section denies unknown fields.
- **X11:** pure Rust GetImage (no XShm dependency), primary RandR monitor rectangle, whole-root fallback, XTest events, XFixes cursor. Real Xvfb tests cover the full tool path. The root visual's channel masks, byte order and scanline padding are honored.
- **macOS:** direct small CoreGraphics/CoreFoundation/ApplicationServices FFI declarations avoid wrapper dependencies and SDK build scripts. Screen Recording/Accessibility preflights provide actionable errors. CGDisplayCreateImage deprecation is a future compatibility risk. ASCII combos use US hardware positions; Unicode text is layout-independent. F21–F24 have no standard Mac hardware codes and fail clearly; media keys are explicitly unsupported: ordinary CG keyboard events do not establish reliable system-defined volume/media behavior.
- **Windows:** target-gated windows-sys, GDI BitBlt rather than DXGI, cursor arrow fallback, SendInput including virtual-desktop normalization. Process DPI setup runs once and every operation verifies the effective thread PER_MONITOR_AWARE_V2 context; conflicting contexts fail clearly. VkKeyScanW Shift/Ctrl/Alt requirements are acquired only if not explicitly held and released by their owner, using remembered press mappings. Primary monitor capture only. UAC and secure desktops remain inaccessible.
- **Wayland:** ashpd with Tokio/zbus, no libdbus or libpipewire development libraries. RemoteDesktop requests one monitor plus keyboard/pointer devices; portals cannot identify or force the primary monitor, so the user must have a single-monitor session. Persistence is managed only through SelectDevices; a small call through ashpd's public zbus proxy omits persistence options from combined SelectSources (the typed ashpd 0.12 API always includes them). Restore tokens use an absolute XDG_STATE_HOME or HOME base, an owned 0700 application directory and atomic 0600 files. Symlink/nonregular/insecure token files are ignored, whitespace is trimmed, and persistence failures warn without denying the active session. Sessions are explicitly closed on setup failure/timeout and backend drop. Portal setup stages and Screenshot consent are bounded to 60 seconds; close attempts are also bounded. Native session info reads only atomic cached dimensions and returns screenshot guidance before capture. Every pointer operation refreshes Screenshot geometry before mapping, then verifies current capture/stream ratios before injecting; >1% disagreement or dimensions changing mid-operation fails clearly. Matching ratios cannot prove the selected stream and Screenshot cover the same display. Safe portal temporary-file reads require an owned, single-link regular file with O_NOFOLLOW/O_NONBLOCK; reads are capped at 256 MiB, PNG decoding at 64 megapixels and a 256 MiB decoder budget. Accepted temporary files are removed after reads, including oversized/read-error cases, only if path identity still matches. Unsafe files are left untouched.

### Dependencies

`x11rb` is Linux-only, pure Rust, with xtest/randr/xfixes extensions and no allow-unsafe-code or libxcb features. `ashpd` is Linux-only with defaults disabled and Tokio enabled: the required portal client adds zbus and URL support but no C development libraries. Linux-only `libc` supplies O_NOFOLLOW/O_NONBLOCK and effective-UID checks for safe portal-file access; it was already transitive. Linux `tokio` supplies the portal runtime; `futures-util` (already in ashpd's graph, defaults off) consumes session Closed signals. `windows-sys` is Windows-only with the four required API families. macOS uses framework FFI directly without new crates. `png` and `jpeg-encoder` are small format-specific shared codecs; `jpeg-decoder` is a dev-only round-trip check, defaults disabled. No image umbrella crate, clipboard library, AWS SDK or network service was added.

### Remaining M2 deviations / real-session risks

Wayland Screenshot does not identify monitor geometry or guarantee a primary-only image: it may capture the entire desktop, while input is relative to the chosen stream. **Use one monitor on Wayland for this implementation.** Mixed-DPI/multi-monitor portal capture needs PipeWire frames or compositor-specific geometry before it can be correct generally; neither is claimed implemented. Cursor metadata/exclusion and full Unicode portal input need GNOME/KDE testing. Native dimensions on Wayland require a first authorized screenshot; get_session_info returns guidance until then, unlike the backend-known size on macOS/Windows/X11 startup. Screenshot-to-stream scaling remains valid only if the screenshot represents the selected single monitor. Ratio rejection detects some mismatches, not same-aspect multi-monitor arrangements. This backend is experimental and does not fulfill the general primary-monitor or cursor-exclusion contract. The portal may require renewed consent despite a restore token.

Native key availability depends on the hardware/keymap. Fn uses a private internal identifier and is rejected on X11/Wayland/Windows, never substituted with XF86WakeUp. macOS uses direct framework FFI rather than the preferred objc2/core-graphics wrapper; declarations compile without the SDK but still need a real macOS runner to link and run. macOS/Windows have ignored capture smoke tests; full GUI action tests are manual. X11 integration lives in taa-tools (the owner of mapping/encoding) to exercise public execute without a platform→tools dependency cycle. Wayland integration is manual because this box has no compositor/portal.

## Roadmap

M3 sequences and screenshot saving, M4 stdio MCP forwarding, M5 five computer-use adapters, and M7 local replay and human stop button are implemented. Authentication comes later. Windows SendInput/GDI is included in M2 rather than deferred to M6; DXGI remains optional future work.

## MCP tool forwarding (M4)

The normative AWS behavior is [aws-parity.md §8](aws-parity.md#8-mcp-tool-forwarding).
`taa-forward` is an Axum-independent crate: it parses the redirection file, spawns
stdio children inheriting the parent environment, and multiplexes newline-delimited
JSON-RPC responses by numeric request ID. A reader task dispatches oneshot responses;
forwarded calls never acquire the desktop lock. Unknown JSON configuration keys are
ignored with stderr warnings. Invalid encodings (including every BOM), JSON or shape
reject the whole file. Configuration order is preserved. Names are rewritten as
`forwarded___{server}___{tool}` without validation; ambiguous names warn, and exact
manifest mappings route calls without splitting names. Empty or failed listings
contribute no tools. Native sequence validation continues to reject forwarded actions.

| Budget | Default | Scope |
|---|---|---|
| Request | 5 seconds | Each downstream call, or all pages of one listing; timeout sends notifications/cancelled |
| Environment teardown | 15 seconds | All children concurrently |
| Graceful child close | 1 second | Close stdin, wait, then kill and reap |
| Initialize safety cap | 60 seconds | Local cap, independent of request budget; PROVISIONAL (not published by AWS) |

`kill_on_drop` backs up explicit child teardown. Timeouts are injectable for tests.
Downstream CallToolResult objects pass through unchanged, including structuredContent,
_meta and isError. Locally generated failures contain one text block and isError:true:
JSON-RPC -32600/-32601/-32602 become `CLIENT_ERROR: <message>`; other codes become
`SERVER_ERROR: <message>`. Timeouts say `SERVER_ERROR: forwarded tool call timed out after 5s`.
Transport failures say `SERVER_ERROR: forwarded server '<name>' is unavailable`.
Non-object arguments produce CLIENT_ERROR; absent arguments become an empty object.
Arguments are not checked against downstream schemas.

Every local policy below is **PROVISIONAL (not published by AWS)**:

- `[forwarding] config_path` overrides the redirection file location; nonempty paths
  must be absolute. Empty selects the documented Linux/Windows defaults. The macOS
  default is `/Library/Application Support/tinyagentaccess/mcp_server_redirection_config.json`.
- Each stateful or stateless mcpSessionId session owns a lazy environment. The file
  is read once on first list/call; initialization starts then. Invalid/missing files
  warn once for that session and leave native tools available. Spawn/initialize
  failures skip that server. No child is spawned when forwarding is disabled.
- Stateless discovery without an existing mcpSessionId uses a temporary environment,
  torn down in a task after listing; it does not consume a session slot. Discovery
  with `arguments.mcpSessionId` uses that session. `NEW` on a forwarded call creates
  a normal session environment. This accommodates session-free modern discovery.
- DELETE, close_session and idle expiry schedule teardown outside the HTTP response
  path. Idle expiry is polled once per second and also checked during admission.
  Graceful process shutdown tears down environments before draining HTTP requests.
- Initialize offers 2025-06-18, empty capabilities, clientInfo tinyAgentAccess and the
  cargo version; known MCP versions returned by downstreams are accepted, followed
  by notifications/initialized. A 60-second per-server safety cap bounds hangs.
- Each upstream list refreshes every server, follows nextCursor within one request
  budget, and rejects repeated cursors. Successful manifests replace that server's
  cached routing map; a failed refresh exposes no tools but preserves the last
  successful map for calls. Concurrent route misses share a completed refresh; explicit lists always re-list.
  Unknown forwarded names trigger at most one refresh before
  CLIENT_ERROR Unknown tool. Collisions choose the first entry in config order and warn.
- Exited children are unavailable to calls and respawn lazily at most once during
  each upstream listing. A refresh triggered by an unknown name counts as a listing.
- Tool objects pass through verbatim except name. Modern schemas additionally receive
  the native mcpSessionId string property and required entry; that argument is stripped
  before forwarding. Modern results receive the usual session block and result metadata.
- The result/error relay and argument policy described above are local decisions;
  downstream isError:true text is never rewritten.
- Stderr is forwarded line by line with `[forwarded:<name>]`; non-JSON stdout warns
  and is ignored. Server requests (sampling/roots/elicitation) receive -32601;
  notifications, including tools/list_changed, are ignored because lists refresh.

The Rust fixture is a normal binary so Cargo provides CARGO_BIN_EXE to integration
tests and SDK verification can reuse it without Python in cargo tests. taa-mcp builds
the same source as a separately named fixture binary for transport tests.

Forwarding refreshes are serialized by a dedicated refresh mutex. Discovery snapshots
entries under the routing-state mutex, performs spawn/initialize/list outside it,
and briefly reacquires it to publish. Cached healthy routes remain callable during
discovery. Teardown marks the set closed and interrupts refreshes without waiting
for discovery; children being initialized or listed are tracked separately so
teardown can close and reap them before publication. Kill-on-drop remains a backstop. Interrupted downstream
frame writes close the pipe and mark the client unavailable. Timeout cancellation
is sent only after the complete request frame was written.

## M5 computer-use dialect adapters

All five dialects are **best-effort, unverified vs AWS**. The registry lives in
`taa-tools::dialects`, with Anthropic, OpenAI and Google modules. Translation produces
one native call, then the HTTP handler uses its existing desktop lock, session
platform (including held-button ownership), validation, execution and screenshot
saving. Native execute_sequence remains unchanged and takes native actions only.
The interface header is read on each list/non-forwarded call; nothing
is stored in a session. Only the 14 standard desktop tools are replaced. Existing
session/sequence/wait tools and forwarded tools retain their schemas and behavior;
future catalog/session native tools are retained by the filter.

Each local choice below is **PROVISIONAL**, not an AWS-confirmed behavior:

- Forwarded calls ignore the interface header entirely, including unknown values and
  calls when forwarding is disabled. This prioritizes §8 interface independence over
  §6 unsupported-interface rejection; the spec does not say which rule wins.
- All five input schemas are vendor-derived approximations, not AWS-confirmed.
  Anthropic's single tool uses `action`, coordinates as pairs, and text for keys;
  its toolset splits these into individual functions. This permits use with
  vendor clients until captured AWS schemas are available.
- ANTHROPIC_COMPUTER_20250124 uses the same single `computer` adapter as 20251124,
  including its action set, because §7.3 gives no schema or mapping. Shared code
  makes this easy to replace when evidence arrives.
- Stateless dialect schemas require `mcpSessionId`, just like native tools;
  translation strips it and results use the existing session envelope. AWS support
  is unknown; this preserves usable local stateless behavior.
- Anthropic scroll amounts and Gemini `magnitude` are notches multiplied by 120,
  using checked integer arithmetic; schema-enumerated lowercase directions are canonicalized
  to native title case. This uses native documented ticks without overflow.
- Key conversion preserves xdotool names and normalizes Control/CTRL to ctrl,
  Command/cmd/Meta/Win to super, Option to alt, Shift, Enter/Return, Esc/Escape and
  Arrow direction aliases. OpenAI/Gemini key arrays are joined with `+` before
  conversion; empty/non-string arrays fail. Unrecognized names reach native key
  validation rather than inventing a platform key vocabulary.
- OpenAI uses x/y pixels, text, keys[], button (default left), and path[]. Drag
  paths require at least two integer points and reduce to first/last endpoints,
  since the native action is an endpoint drag; intermediate points are not executed.
- OpenAI scroll_x/scroll_y are signed native ticks (positive right/down); absolute
  magnitude is checked. Both fields are required. Diagonal scroll is rejected
  before dispatch so callers can issue separate axis calls without partial execution.
  Zero deltas on both axes are rejected with CLIENT_ERROR instead of dispatching a
  zero-tick scroll; the spec does not define this case.
- OpenAI wait is rejected with CLIENT_ERROR guidance to agentaccess___wait, resolving
  the contradictory sample in favor of the documented dedicated pause tool.
- Gemini uses x/y, destination_x/destination_y for drag, text for type, key for
  press_key, keys[] for hotkey, and direction/magnitude for scroll. The spec leaves
  argument names unknown; these isolated vendor-derived adapters define our local
  contract without asserting AWS schema parity.
- Gemini grid conversion uses nearest integer `n * (extent - 1) / 999` (integer
  rounding), making 0 and 999 map to the first and last valid pixel. It uses the
  configured screenshot coordinate space (1280×720 or cached native dimensions).
  Unknown dimensions and grid values outside 0–999 fail; fresh native geometry is
  still validated at execution. This avoids out-of-bounds inclusive endpoints.
- Dialect argument shape failures and hidden/unknown tools return existing
  CLIENT_ERROR tool results. Rejected Anthropic wait/zoom/cursor_position actions
  point to agentaccess___wait even though they are absent from listings. Adapter
  schemas reject unknown properties; native semantic validation remains authoritative.
- Permission filtering preserves screenshot-only access through single computer
  tools while refusing translated input when disabled; split tools are filtered
  by their corresponding native permission. AWS hidden-tool behavior is unknown.

## M7 local replay and human control

The source facts are [§4 and §9](aws-parity.md). AWS Replay Session is public preview,
stacks only, and not listed in API_AgentAccessConfig [D-evidence]. It captures a
whole-screen frame and tool/arguments/response/timestamp just before actions, then
uses Bedrock, S3, KMS and cross-Region inference for its video/transcript/HTML report
with ≤15-day AWS retention. Those services are excluded here. AWS observers use a
streaming URL; VIEW_STOP ends access, requiring a new stack session or an image-builder
stop/start [D-session, D-api-cfg]. We do not model image builders or streaming URLs.

Each following bullet is a separate **PROVISIONAL / TINY-PROPOSAL** local choice:

- **[D-evidence] Configuration:** `[replay] enabled=false` and `save_dir=""` are local,
  denied-unknown-key fields. Enabling requires a nonempty absolute path because no
  S3/KMS configuration applies. Replay is independent of returned screenshot saving.
- **[D-evidence] Timing:** `replay::Context` captures the frame and writes a `before`
  JSONL record immediately before native dispatch; `Event::finish` appends an `after`
  record with the same sequence and the response on success or error. A response
  cannot exist before dispatch; the paired-record policy is isolated for replacement.
  UTC timestamps use explicit Unix epoch milliseconds, not local wall time.
- **[D-evidence, D-monitor] Layout/privacy:** a session's first event fixes its UTC
  directory `agentaccess/replay/year=YYYY/month=MM/day=DD/{mcp-session-id}`;
  `{seq}-{epoch-ms}.png|jpg` uses the actual configured format. Session components
  use screenshot-saving's ASCII alphanumeric/hyphen validation. Sink directories
  reject symlinks; files use exclusive creation. Unix created directories/files use
  0700/0600, Windows inherited ACLs, with the same best-effort path-race posture as
  screenshot saving. Existing configured ancestors may be system aliases.
- **[D-evidence] Capture:** replay reuses the normal backend/render/encoding path
  with `include_cursor=false` and configured resolution/format under the existing
  desktop lock. This means the backend's whole-screen scope and existing Wayland
  limitations apply. Capture/storage failure warns to stderr and leaves execution
  and MCP results unchanged; a failed capture can yield metadata without a frame.
- **[D-evidence] Coverage:** desktop calls record client name/arguments and native
  action/arguments; sequences record each dispatched native step, not skipped steps
  or their synthetic terminal screenshot. Wait is metadata-only to avoid unnecessary
  capture. `get_session_info`, `close_session` and forwarded calls record metadata
  only; forwarding retains its independent admission and never takes the desktop lock.
  Initialize, list, raw HTTP frames and observer polls are not replay events.
- **[D-evidence] Bounds:** each argument/response JSON payload over 65,536 UTF-8
  bytes is replaced with an explicit truncation marker, original byte count and
  first 16,384 Unicode scalars (at most 65,536 bytes). Image-response base64 is omitted
  because the pre-action image is a separate artifact. This limits opaque forwarded
  payloads; desktop text and screenshots may contain sensitive information. No
  general redaction or retention deletion is promised; users own local cleanup.
- **[D-evidence] Named secrets:** the local metadata sanitizer recursively omits common
  password/token/API-key/authorization/credential fields before truncation, to avoid
  retaining explicitly named secrets from forwarded tools. This is best effort;
  arbitrary text and desktop pixels are not redacted. AWS documents no redaction.
- **[D-evidence] Finalization:** close_session, DELETE, idle expiry and observer Stop
  finalize an escaped, script-free local `index.html` when events exist. The index
  labels itself TINY-PROPOSAL, not an AWS/Bedrock report; no video or transcript is
  generated. Pending forwarded responses defer finalization until completion.
- **[D-evidence] Cancellation:** if the HTTP future disappears before a recorded
  response exists, an explicit cancellation/unknown-outcome record is written; native
  blocking operations already admitted may still complete. This does not invent a
  successful/error tool result. The normal desktop-lock cancellation limitations
  remain those of the existing worker path.
- **[D-session, D-api-cfg] Observer substitute:** `/observe` serves a static, same-origin
  page; `/observe/sessions` lists live IDs, `/observe/latest/{id}` returns a fresh MCP
  image JSON, and POST `/observe/stop/{id}` closes that ID. Polls are sequential at
  1.5 seconds after each completed request. This is local polling, not DCV or an
  AWS streaming URL. All sessions share the local desktop. Vision-disabled servers
  reject image capture; observation uses normal capture/consent and pins active work.
- **[D-session] Mode policy:** disabled routes return 404; view_only has no button
  and rejects Stop; view_stop allows Stop only for live or remembered closed IDs,
  returning 204 idempotently and 404 for unknown IDs. The shared release/close path
  releases held buttons, removes ownership/session state and tears down forwarding.
  Stop waits for admitted desktop work, including an entire sequence, to finish;
  queued requests recheck existence and fail after closure. No local image-builder
  stop/start or OS logout exists. A new MCP session is required to resume.
- **[D-session] Browser safety:** observer responses disable caching; the page denies
  framing and external resources. Existing Host/Origin checks also apply, with exact
  Origin/Host authority matching and a required Origin for Stop. No auth is added;
  these are browser checks, not identity. Keep the unauthenticated bind on loopback.
- **[S-retry, D-session] Disconnect notification:** no `client_disconnected` wire
  message is emitted because its exact shape is UNKNOWN and GET/SSE is unsupported.
  Session closure works independently, preserving close_session and HTTP DELETE.
