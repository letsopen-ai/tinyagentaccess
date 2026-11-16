# tinyAgentAccess milestone 1 design

This is a local implementation, with no AWS SDK, credentials, cloud connection, TLS stack, or authentication. The evidence source is [aws-parity.md](aws-parity.md); this document records implementation choices, not additional claims about AWS.

## Architecture and request flow

`tinyagentaccess` reads TOML and CLI overrides, validates configuration, binds a Tokio listener, and serves the `taa-mcp` Axum router at `/mcp`. `taa-mcp` checks Host/Origin, parses JSON-RPC, chooses protocol/session, checks the per-request interface, and calls `taa-tools`. `taa-tools` owns native schemas, gating, validation, session-independent waits, and sequential desktop dispatch. `taa-platform` provides an object-safe `Platform` boundary for screenshots, screen size, movement/click count/button/modifiers, button down/up, drag, directional wheel ticks/modifiers, key combinations, holds, and text.

The executable always uses `UnavailablePlatform`: desktop calls return `isError: true` and `SERVER_ERROR: desktop backend not available on this platform yet`. `MockPlatform::recording()` records dispatch for tests and supplies a synthetic empty image payload; it is not a user desktop or a valid screenshot source. Platform functions are synchronous in M1; future slow desktop implementations need blocking-worker isolation.

## Transport and sessions

Stateful versions are 2025-03-26, 2025-06-18, and 2025-11-25. `initialize` echoes a supported request or selects 2025-11-25, returns tools-only capabilities (`listChanged: false`), serverInfo `tinyAgentAccess` and the Cargo version, plus a random UUID `Mcp-Session-Id`. Later POSTs require it, including initialized notifications (202 empty) and ping. DELETE returns 204; unknown/expired/closed IDs return HTTP 404. The protocol header, when supplied, must match the session. Unsupported POST protocol headers return structured HTTP 400 / JSON-RPC -32022, regardless of method. DELETE retains the legacy transport-only HTTP 400 for unsupported headers.

The installed official Python MCP SDK 2.1.1 confirms the 2026-07-28 wire protocol: `server/discover`, no initialize, and no transport session ID. Discovery returns `supportedVersions`, capabilities, `resultType: complete`, `ttlMs: 0`, `cacheScope: private`, and display-only `_meta["io.modelcontextprotocol/serverInfo"]`. Each modern request carries `_meta["io.modelcontextprotocol/protocolVersion"]`, `_meta["io.modelcontextprotocol/clientCapabilities"]`, optional clientInfo, and HTTP `MCP-Protocol-Version`, `Mcp-Method`, and (tools/call) `Mcp-Name`. Header mismatch is -32020. Modern list results include required cache fields; tool results include `resultType: complete`. Plain `application/json` responses are accepted by the SDK; no SSE is needed. GET and modern DELETE return 405.

Unsupported POST versions (including discovery) receive JSON-RPC -32022 (`UNSUPPORTED_PROTOCOL_VERSION`), with `data.supported` and `data.requested`, matching SDK negotiation. This code is not used as a generic AWS error. Discovery and tool listing do not allocate desktop sessions.

Stateless calls require `arguments.mcpSessionId`. NEW creates a desktop session; later calls reuse its UUID. Every modern tool result includes both `result.mcpSessionId` and a JSON text block containing the ID, including failures and close. JSON-RPC errors associated with a supplied existing ID include `error.data.mcpSessionId`. HTTP/security errors and malformed requests are transport failures, not tool results.

[TINY-PROPOSAL] Both kinds share `max_sessions` (default 1). Refusal uses `CONNECTION_LIMIT_REACHED: Another agentic client is already connected`: stateful initialization returns JSON-RPC -32000, stateless NEW returns a CLIENT_ERROR tool result. Unknown stateless IDs return `CLIENT_ERROR: Unknown mcpSessionId`. Missing ID returns `CLIENT_ERROR: mcpSessionId is required` with an empty ID marker; refused NEW retains NEW because no ID was allocated. Close returns `Session closed`, including repeat close for a remembered ID. Closed IDs are remembered for a 300-second retry window, then lazily pruned on session access/allocation. Idle timeout defaults to 780 seconds, pruned lazily on session access/allocation; expired IDs are unknown, not automatically reconnected. A shared async mutex serializes tool dispatch across desktop sessions. Session existence is checked again after acquiring it, so queued work cannot execute after closure. Admitted work is pinned against idle expiry until completion or cancellation; completion restarts its idle timer. Closure does not interrupt work already admitted. Queued work may expire before admission. This is deliberately simpler than AWS reconnect semantics.

## Security

Default bind is `127.0.0.1:8765`. `--bind` overrides TOML. Wildcard binds (`0.0.0.0` and `::`) fail configuration validation with a no-auth safety error; use a concrete IP socket address. A concrete non-loopback bind is allowed and emits a loud no-auth warning. Host must name localhost, a loopback IP, or the explicitly bound non-loopback IP. Arbitrary DNS names are rejected to reduce DNS rebinding exposure. Origin, if present, must use http/https and a loopback IP or localhost authority; other origins, null, userinfo, paths, and duplicate Host/Origin headers return 403. POST requires Content-Type application/json (parameters allowed), returning 415 otherwise. This JSON-response-only server requires Accept application/json (application/* and */* allowed), returning 406 otherwise; SSE acceptance is not required, matching the SDK JSON-response mode. Duplicate or non-UTF-8 routing headers and duplicate Content-Type return 400. Host/Origin remain 403 (an intentional difference from the SDK’s 421 for bad Host). Unknown AWS headers and query parameters are ignored. These checks do not authenticate local clients. Never expose this server.

## Registry, validation, outputs and errors

[TINY-PROPOSAL] All schemas are provisional, marked at the schema generator, and are not AWS-verified. Objects reject extra argument keys, coordinates are signed 64-bit JSON integer representations in [0,width) / [0,height) (2.0 is deliberately rejected; AWS integral-float acceptance remains unknown), scroll_amount is an integer without a speculative sign restriction, and modifiers/key combinations remain strings without an invented key vocabulary. Unicode scalar values count toward text limits. InputSchema publishes documented duration, text, direction, and action-count limits. The execute_sequence maxItems constraint is enforced by its bespoke preflight to retain sequence-summary rejection diagnostics; the generic property validator is deliberately not a full JSON Schema engine. Modern schemas require mcpSessionId. Names use agentaccess___; short names are accepted only within sequences.

[TINY-PROPOSAL] Gated tools are hidden and calls return `CLIENT_ERROR: Unknown tool` with `isError: true`, as do unknown names. Vision gates screenshot; input gates mouse/keyboard and execute_sequence. wait/get_session_info always exist; close_session is listed only in stateless mode. Input without vision is a config error.

[TINY-PROPOSAL] Missing stateful header is HTTP 400 plain UTF-8 text exactly `Missing Mcp-Session-Id header`. JSON-RPC parse/invalid-request/method/params errors use -32700/-32600/-32601/-32602; internal/interface-not-implemented errors use -32603. Modern JSON-RPC errors map parse/invalid-request/invalid-params/header-mismatch/unsupported-version to HTTP 400 and method-not-found to 404; internal errors retain HTTP 200. Legacy parse/envelope errors return HTTP 400; handler method/parameter errors remain JSON-RPC responses with HTTP 200. Invalid request IDs return null, never echo untrusted arrays/objects. Params must be an object. Valid modern notifications are acknowledged and dropped with 202 using only the version header, without requiring request metadata/routing headers, matching the SDK.

POST bodies have an explicit 2 MiB cap (413), preserving Axum’s existing Bytes default. serde_json’s default 128 nesting limit remains enabled; excessive nesting produces a parse error, not recursive dispatch. Batches are accepted within established 2025-03-26 sessions, excluding initialize, and notification members produce no result. Nested arrays and other non-object members receive -32600 member errors; dispatch never recursively invokes the HTTP handler. Other versions and empty arrays reject batches with -32600. Receiving batches is required by the [2025-03-26 base protocol](https://modelcontextprotocol.io/specification/2025-03-26/basic#batching); later versions removed batching. Native interface is omitted/empty. The five known case-sensitive interface IDs are represented by an enum and return -32603 with `Computer-use interface is not implemented yet`; other values return -32602. No silent dialect fallback.

[TINY-PROPOSAL] wait sleeps using Tokio and succeeds with `Wait completed`; invalid duration returns the CLIENT_ERROR text `wait duration must be a number between 0.001 and 5.0 seconds`. Desktop successes use `Action completed`. Tool failures use isError and CLIENT_ERROR / SERVER_ERROR prefixes. Screenshot output uses an MCP image block with base64 data and platform MIME type; real format conversion/scaling is deferred with capture.

execute_sequence validates the whole sequence before dispatch: ≤20 actions, per-wait 0.001–5 seconds, cumulative waits ≤10 seconds and text ≤10,000 characters, no screenshot or recursive sequence. Session tools are also excluded [TINY-PROPOSAL]. Rejection returns JSON text with sequence_summary (submitted/executed/succeeded/stopped_at_index/stop_reason/guidance) and skipped action_results; a second block contains `CLIENT_ERROR: Rejected before dispatch: ...`. Guidance is verbatim: `Correct the sequence and retry. No desktop action ran, so no screenshot is needed.` Documented rejection reasons preserve AWS sample wording. [TINY-PROPOSAL] stopped_at_index is the offending index, or the last checked index for cumulative limits (0 for action-count overflow).

Execution is minimal: sequential dispatch, first failure stops later actions, 60-second admission deadline checked before each action (in-flight actions are not interrupted). Success requests a final screenshot. No extra stabilization pause in M1. Failure/deadline returns text only with guidance `Take a screenshot before retrying.` Rich terminal diagnostics, timing polish, and stabilization are milestone 3. With the unavailable backend even a wait-only sequence fails its terminal screenshot, while standalone wait works.

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
| agent_access.forward_mcp_tools = false | Settings FORWARD_MCP_TOOLS DISABLED; accepted no-op until M4 |
| agent_access.screen_resolution = "1280x720" | ScreenResolution W_1280xH_720; local extension "native" |
| agent_access.screen_image_format = "png" | ScreenImageFormat PNG / JPEG, lowercase TOML png/jpeg |
| agent_access.user_control_mode = "disabled" | UserControlMode DISABLED / VIEW_ONLY / VIEW_STOP mapped to disabled/view_only/view_stop; no-op until human controls |
| screenshots.save_dir = "" | local replacement for ScreenshotsUploadEnabled/S3BucketArn; no-op until M7; never S3 |
| bind / sessions.max_sessions / sessions.idle_timeout_secs | local transport/session settings; 780-second default inspired by image-builder timeout |

AWS requires at least one enabled action; [TINY-PROPOSAL] local all-disabled is allowed for session/wait-only operation. ReplaySessionEnabled/KmsKeyArn have no local config yet.

## Platform plan and roadmap

M2: macOS ScreenCaptureKit / CoreGraphics capture with CGEvent input; Screen Recording and Accessibility permissions required. Linux Wayland uses xdg-desktop-portal ScreenCast + RemoteDesktop (for example ashpd), GNOME/KDE consent dialog once with restore tokens to avoid re-prompts, with X11 fallback via XTest + XShm/XGetImage. Capture scales to 1280×720 and maps agent coordinates back to native; native mode skips scaling. Format selection/coordinate mapping belongs in the backend boundary.

M3: execute_sequence engine, stabilization waits, deadlines and session polish. M4: MCP forwarding. M5: five computer-use interface adapters. M6: Windows SendInput + DXGI Desktop Duplication. M7: local screenshot saving, replay, human stop button. Authentication comes later. No milestone 1 desktop functionality is implied by a successful transport test.
