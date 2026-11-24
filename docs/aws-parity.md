# AWS WorkSpaces "agent access" MCP server (agentaccess-mcp): functional spec for tinyAgentAccess

Status: researched Nov 16, 2026. Revision 2: revises an earlier draft after checking it line by line against the AWS docs and extending it.
Scope: functional behavior to reproduce in tinyAgentAccess (Rust, letsopen-ai org). Section 12 covers AWS authentication and the local token-file departure.

tinyAgentAccess is independent and is not affiliated with, endorsed by, or sponsored by Amazon Web Services or Amazon. Amazon WorkSpaces, AWS, and related marks are trademarks of Amazon.com, Inc. or its affiliates. This document independently summarizes and paraphrases AWS public documentation with citations; the [official AWS developer guide](https://docs.aws.amazon.com/appstream2/latest/developerguide/agent-access.html) is authoritative and remains Amazon’s copyright. See [§16](#16-sources) for sources; the AWS sample repository is MIT-0.

## 0. How to read this document (evidence levels)

Each claim is tagged with where it comes from:

- **[DOC]**: AWS documentation (docs.aws.amazon.com) or an AWS What's New / product page. Normative.
- **[SAMPLE]**: AWS's official sample repo `aws-samples/sample-code-for-workspaces-agent-access` (MIT-0, commit `19839c7`, Nov 13, 2026). It shows observed service behavior and the client's assumptions. It is **not** an API contract, and it disagrees with the docs in a few places (flagged).
- **UNDOCUMENTED / UNKNOWN**: neither source states it. Do not treat it as AWS behavior.
- **[TINY-PROPOSAL]**: a choice we make for tinyAgentAccess where AWS says nothing. Always labeled; never presented as AWS behavior.

Source keys (full URLs in §16): D-tools, D-cui, D-endpoint, D-session, D-connmode, D-cleanup, D-prereq, D-forward, D-monitor, D-evidence, D-getstarted, D-ident, D-ident-ndj, D-ident-dj, D-concepts, D-llms, D-bp, D-regions, D-api-cfg, D-api-set, D-api-csu, D-api-cibsu, D-sar, D-reg, W-ga, W-oct, W-nov, P-product, S-readme, S-agentsmd, S-cui, S-ctool, S-mcpclient, S-djauth, S-seq, S-quick, S-retry, S-fwd-contract, G-proxy.

---

## 1. Product identity

- Official names: **"Amazon WorkSpaces for AI agents"** (marketing/What's New) [W-ga, P-product]. In the docs it is **"agent access"**, a feature of **Amazon WorkSpaces Applications** (formerly AppStream 2.0; the docs live under `/appstream2/`): the developer guide identifies WorkSpaces for AI agents as the agent access feature of Amazon WorkSpaces Applications [D-learn-more]. The IAM service name is **"Amazon WorkSpaces AgentAccess MCP Server"**, with service prefix `agentaccess-mcp` [D-sar].
- What it is: a fully managed, AWS-hosted MCP server. It gives AI agents tools to operate desktop applications in WorkSpaces Applications streaming sessions (click, type, scroll, screenshot), plus forwarding of MCP servers that run on the desktop [D-mcp-server, D-concepts].
- Closed source. No server code is published. The only official code is the client-side sample repo [S-readme].
- Timeline [DOC]:
  - GA on Jun 30, 2026 [W-ga].
  - Oct 20, 2026 [W-oct]: OAuth 2.0, service-managed connections (stack + user headers instead of streaming URLs), session-state query, the app catalog tools, and 4 more Regions (sa-east-1, eu-south-1, eu-south-2, ap-southeast-5).
  - Nov 3, 2026 [W-nov]: Linux and Windows 11 desktops. Linux gives agents "full root access".
  - **Correction vs earlier draft:** the Oct 20 update was missing from the draft.

## 2. Architecture and transport

- A remote MCP server over **Streamable HTTP** only [D-endpoint]. No stdio and no SSE endpoint is documented. (`mcp-proxy-for-aws` can expose it locally as stdio [S-agentsmd, G-proxy].)
- Endpoint: `https://agentaccess-mcp.{region}.api.aws/mcp` [D-endpoint].
- 17 Regions [D-regions]: ap-northeast-1, ap-northeast-2, ap-south-1, ap-southeast-1, ap-southeast-2, ap-southeast-5, ca-central-1, eu-central-1, eu-south-1, eu-south-2, eu-west-1, eu-west-2, eu-west-3, sa-east-1, us-east-1, us-east-2, us-west-2. (The sample's AGENTS.md lists only 13 and is out of date [S-agentsmd].)
- Data flow [D-concepts]: agent → MCP server → WorkSpaces Applications desktop "over a secure channel". The sample's errors and paths name NICE **DCV** as the remote-desktop channel ("DCV proxy not initialized", "dcv session not ready", `C:\ProgramData\NICE\dcv\...`) [S-agentsmd, D-forward].
- MCP protocol versions [D-endpoint]:

| Version | Type |
|---|---|
| 2025-03-26 | Stateful |
| 2025-06-18 | Stateful |
| 2025-11-25 | Stateful |
| 2026-07-28 | Stateless |

  - **Stateful versions**: the session is ended by HTTP `DELETE` with `Mcp-Session-Id` [D-cleanup].
    - The `initialize` response carries an `Mcp-Session-Id` header that must be sent on every later request.
    - A request without it gets **HTTP 400 `Missing Mcp-Session-Id header`** [D-ident-dj].
  - **Stateless (2026-07-28)**: the session is ended by the `close_session` tool [D-endpoint, D-tools].
- Version negotiation: the sample's stateless client falls back to stateful when the server does not offer 2026-07-28. It also handles a JSON-RPC error code **`-32022`** (it calls this "the version fallback") [S-mcpclient]. The exact meaning of -32022 is UNDOCUMENTED.
- MCP capabilities used: tools only. **MCP resources and prompts: UNDOCUMENTED.** No doc or sample mentions them; assume none.
- `tools/list` pagination: the sample follows `nextCursor` pages [S-seq]. Whether AWS actually paginates is UNKNOWN.
- No VPC endpoints (PrivateLink) [D-prereq].

## 3. Request headers, query parameters and `_meta` (session targeting)

Header names are case-insensitive. The value of the interface header is case-sensitive [D-cui].

| Header | Meaning | Source |
|---|---|---|
| `x-amzn-agentaccess-streaming-session-url` | Streaming URL from `CreateStreamingURL` (stack) or `CreateImageBuilderStreamingURL`. Sent on **every** request. | D-endpoint |
| `x-amzn-agentaccess-workspaces-applications-stack` | Stack name **or ARN**. The server creates the streaming session. | D-endpoint |
| `x-amzn-agentaccess-workspaces-applications-user-id` | Optional. Default: the caller's `SourceIdentity` if present, otherwise the IAM principal. With agent identity it must match the SourceIdentity or the request is rejected. User name 2–32 chars. | D-endpoint, D-ident-ndj |
| `x-amzn-agentaccess-workspaces-applications-image-builder-name` | Image builder name. The server creates the session. | D-endpoint |
| `x-amzn-agentaccess-local-user` | **Required** for image builders, no default: `ADMINISTRATOR` \| `TEMPLATE_USER` \| `TEST_USER` | D-endpoint |
| `x-amzn-agentaccess-connect-mode` | `BLOCKING` (default) \| `POLLING` | D-connmode |
| `x-amzn-agentaccess-expire-streaming-session-on-delete` | `true` \| `false` (default `false`). Acts on the stateful HTTP DELETE. | D-cleanup |
| `x-amzn-agentaccess-computer-use-interface-id` | Tool dialect, chosen per request (§7). | D-cui |

- Query parameters [D-endpoint]: `login_hint` (OAuth IdP redirect, auth-only), `workspacesApplicationsStack`, `workspacesApplicationsUserId`. They are documented as alternatives you can combine with `login_hint` on the endpoint URL.
- `_meta` keys:
  - Domain-joined fleets [D-ident-dj, S-djauth]: `aws.agentaccess/workspacesApplicationsSamlAssertion` (base64 SAML Response) and `aws.agentaccess/workspacesApplicationsStackArn`, placed in `params._meta` of **every** JSON-RPC request.
  - The service **rejects** a request that also sends these as headers [D-ident-dj].
  - The endpoint page's mcp-proxy example instead shows `metadata={"saml_response":..., "stack_arn":...}` [D-endpoint]. These are proxy-side names. The sample notes they were never tested and only the `aws.agentaccess/...` keys were seen to work [S-readme].
  - **Correction vs earlier draft:** the `aws.agentaccess/...` keys are also in AWS docs (D-ident-dj), not only in the sample.
  - `aws.agentaccess/streamingSessionUrl` in `_meta` is an alternative to the streaming-URL header [S-agentsmd, IDE config example]. SAMPLE-only; not in the docs.
- Precedence when several targeting inputs are sent: UNDOCUMENTED.

**[TINY-PROPOSAL]** tinyAgentAccess v1 has no AppStream backend. Targeting headers should map to a local "desktop provider" (for example a VNC/X11/RDP target ID). Accept the AWS header names so AWS clients work unchanged.

## 4. Session model and lifecycle

- Stages [D-overview]: authenticate → initiate (open session, `tools/list`) → tool calls → end.
- One AI agent per streaming session. One active session per named UserId per fleet. One agent per image builder [D-endpoint, D-prereq].
- The sample's error for a second client is `CONNECTION_LIMIT_REACHED` / "Another agentic client is already connected" [S-agentsmd, S-retry]. The exact error envelope (HTTP status, JSON-RPC code, or tool error) is UNKNOWN.
- Session start (stateful): the first `initialize` that carries a streaming URL, or SAML `_meta` [S-agentsmd].
- **Connect modes** [D-connmode]:
  - `BLOCKING` (default): the server waits until the desktop connection is up before it responds. When `tools/list` returns, all tools are available.
  - `POLLING`: the server responds immediately, and only the `connection_status` tool is listed. The client polls it until the connection is up, after which `tools/list` returns the full set.
  - The docs' client code [D-connmode]: `result = call_tool("connection_status", {})`, then `json.loads(result.content[0].text)["state"] == "CONNECTED"`, retried every 2 s.
  - **connection_status details (everything known):**
    - The result is a text content block holding a JSON object with a `state` field. `"CONNECTED"` is the terminal success value [D-connmode].
    - IAM action `CheckConnectionStatus`: "check the connection status of a streaming session", access level Read [D-sar, D-reg].
    - The docs example calls it **without** the `agentaccess___` prefix [D-connmode]. The tools page says "All tool names use the `agentaccess___` prefix" but does not list connection_status [D-tools]. The real listed name is UNKNOWN.
    - Other `state` values (for example a connecting or failed state), other JSON fields, input schema (the example sends `{}`), the error when the desktop fails, and whether it stays listed after CONNECTED: all UNKNOWN.
    - **Correction vs earlier draft:** the docs are not "inconsistent"; they simply never state the prefix.
- Warm-up: after a streaming URL is created, the DCV session takes 5–30 s, and during that time `tools/call` may return "Unknown tool" [S-agentsmd]. The DJ doc advises allowing time for the desktop connection after `initialize` and retrying a failed initial `tools/list` or tool invocation after a delay [D-ident-dj]. On a stack with application settings persistence, the first `tools/list` waits for the user profile to load, so the sample's HTTP timeout is 300 s [S-mcpclient].
- Image builders: the first connection is slower, so use a client timeout of at least 120 s [D-endpoint]. The MCP session idle-closes after **about 13 minutes**; a subsequent tool invocation reconnects using the user who most recently logged in [D-cleanup].
- Fleet sessions: the MCP session idle timeout is UNDOCUMENTED. The streaming session lives until the fleet's disconnect timeout (when not expired) [D-cleanup].
- **Ending a session**:
  - Stateful [D-cleanup]: HTTP DELETE. With `...-expire-streaming-session-on-delete: true`, a fleet session's streaming instance is terminated (autoscaling applies). An image builder is unaffected.
  - Stateless [D-tools]: `close_session(expire_streaming_session)`, with the same semantics.
  - Image builder: ending the MCP session never logs out the built-in users. Only stopping the builder does [D-cleanup].
  - A session also ends when the streaming URL expires [S-agentsmd]. The sample says the server sends `client_disconnected` when a session is stopped (VIEW_STOP by an observer, URL expiry, timeout) [S-retry]. Its exact wire form is UNKNOWN.
- **Stateless session identity (2026-07-28)**. This comes from the sample; the tools page never mentions it:
  - Every `tools/call` carries an argument **`mcpSessionId`**. The first call passes `"NEW"`, which opens a desktop session. Later calls pass the returned id [S-agentsmd, S-seq, S-quick].
  - The server adds a text content block `{"mcpSessionId": "<id>"}` to **every** stateless tool result, including `close_session` and error results [S-mcpclient, S-seq].
  - When a call fails on an already-created session, the id appears in JSON-RPC `error.data.mcpSessionId` [S-mcpclient].
  - Parallel `NEW` calls open two sessions, so clients serialize [S-quick].
  - Closing an already-closed session "returns closed" [S-mcpclient]. Exact text UNKNOWN.
  - Whether `mcpSessionId` appears in each tool's inputSchema in `tools/list`: UNKNOWN (likely, since models are told to pass it).
  - The sample says computer-use interfaces don't support stateless yet, because the typed tool declarations have no `mcpSessionId` [S-mcpclient]. Treat this as a sample limitation; server behavior is UNKNOWN.
- The streaming URL itself is created with AppStream APIs, not by the MCP server (unless stack/user or image-builder headers are used):
  - `CreateStreamingURL` Validity is 1–604800 s, default 60 [D-api-csu]. UserId is 2–32 chars matching `[\w+=,.@-]*` [D-api-csu].
  - `CreateImageBuilderStreamingURL` default is 3600 s [D-api-cibsu].
  - The console default is 30 min [D-getstarted].

## 5. Native tool catalog (no interface header)

Official list [D-tools]. "All tool names use the `agentaccess___` prefix." Application tools exist only on stacks, and user tools only on image builders. The column "Listed when" combines the docs with sample notes.

### 5.1 What AWS documents per tool (documented facts)

| Tool (`agentaccess___…`) | Purpose [D-tools] | Parameters as documented [D-tools] | IAM action [D-sar] |
|---|---|---|---|
| `left_click` | Left click at coordinates | `x` (req), `y` (req), `modifiers` (opt, e.g. `ctrl`, `ctrl+shift`) | LeftClick |
| `double_click` | Double click | `x`, `y` (req), `modifiers` (opt) | DoubleClick |
| `triple_click` | Triple click | `x`, `y` (req), `modifiers` (opt) | TripleClick |
| `right_click` | Right click | `x`, `y` (req), `modifiers` (opt) | RightClick |
| `middle_click` | Middle click | `x`, `y` (req), `modifiers` (opt) | MiddleClick |
| `left_click_drag` | Drag start→end | `start_x`, `start_y`, `end_x`, `end_y` (all req). **No `modifiers` documented.** | LeftClickDrag |
| `left_mouse_down` | Press and hold left button at coordinates | `x`, `y` (req), `modifiers` (opt) | LeftMouseDown |
| `left_mouse_up` | Release left button at coordinates | `x`, `y` (req), `modifiers` (opt) | LeftMouseUp |
| `move_pointer` | Move pointer | `x`, `y` (req). **No `modifiers`.** | MovePointer |
| `scroll` | Wheel scroll at coordinates | `x`, `y` (req), `scroll_direction` (req, enum `Up`\|`Down`\|`Left`\|`Right`), `scroll_amount` (req, **ticks; 120 = one notch**), `modifiers` (opt) | Scroll |
| `type_text` | Type text by simulating key events for each character | `text` (req, ≤ 10,000 chars) | TypeText |
| `key` | Press key or combination | `keys` (req; single key or `+`-joined, e.g. `a`, `ctrl+c`, `ctrl+shift+s`) | KeyPress |
| `hold_key` | Hold key/combination for a duration | `keys` (req), `duration` (req, 1–30 s) | HoldKey |
| `screenshot` | Capture desktop; all mouse tools use the returned image's dimensions as their coordinate space [D-tools]. | `include_cursor` (opt, default `false`) | GetScreenshot |
| `launch_application` | Launch an app from the fleet image catalog | `id` (req) | LaunchApplication |
| `toggle_app_switcher` | Open/close app-switcher overlay | none | ToggleAppSwitcher |
| `execute_sequence` | Run several tool calls in order | `actions` (req, list of action objects) | ExecuteSequence |
| `wait` | Pause, no desktop interaction | `duration` (req, 0.001–5 s) | Wait |
| `get_session_info` | Current session info | none | GetSessionInfo |
| `close_session` | End session (stateless protocol) | `expire_streaming_session` (opt, Boolean, default `false`) | CloseSession |
| `switch_user` | Image builder: disconnect current built-in user's Windows session, log in another (previous keeps running) | `user` (req, enum `ADMINISTRATOR`\|`TEMPLATE_USER`\|`TEST_USER`) | SwitchUser |
| `get_current_user` | Image builder: logged-in built-in user | none | GetCurrentUser |
| `connection_status` (prefix unknown) | POLLING mode only (§4) | `{}` in example | CheckConnectionStatus |

Additional documented semantics:

- **Key names** (SAMPLE only; the docs give only `a`, `ctrl+c`, `ctrl+shift+s`, `Tab`, `Enter`):
  - Modifiers are `ctrl`, `alt`, `shift`, `super`. Special keys include `Return`, `Escape`, `Tab`, `F1`–`F12` [S-agentsmd].
  - The docs use `Enter` and `Tab` [D-tools]. The sample uses `Return`, `super+r`, `alt+F4`, `alt+Tab` [S-agentsmd, S-readme].
  - The full accepted key vocabulary and its case rules are UNKNOWN. The names look xdotool/X11-keysym style (`Return`, `super`).
- **launch_application** [D-tools]:
  - Listed only when the fleet image has registered catalog apps.
  - The **tool's description lists the valid ids**, filled in by the server from the image catalog (e.g. `firefox`, `notepad++`, `eclipse`).
  - An unknown id returns `isError: true` with text `SERVER_ERROR: Invalid input`.
  - Launching does not close apps that are already running.
  - The Oct 20 What's New says agents can "list the available applications" [W-oct]. No separate list tool is documented; the list lives in the description.
- **toggle_app_switcher** [D-tools]: available only when the fleet's Stream view is **Application**, not Desktop. The sample says it is **not listed** under Desktop view [S-agentsmd]. No tool reports the foreground app.
- **get_session_info** [D-tools]:
  - Stack session fields: `mcpSessionId`, `stack`, `userId`, `workSpacesSessionId`, `fleet`. Image builder fields: `mcpSessionId`, `workSpacesSessionId`, `imageBuilder` (ARN).
  - It does **not** return a streaming URL. The agent's code creates one with `CreateStreamingURL(stack, fleet, userId)` or `CreateImageBuilderStreamingURL(name = last segment of the imageBuilder ARN)`.
  - Then retrieve `streaming_url` from the MCP client's `_meta` and open it in a browser; the URL embeds session credentials, so keeping it in `_meta` keeps it out of the model's context [D-tools].
  - Whether the server itself returns `_meta.streaming_url` on the get_session_info result is **ambiguous / UNKNOWN**. The Oct 20 note says you can "generate a link for real-time observability" [W-oct].
  - The result encoding (JSON text block vs `structuredContent`) and the exact types are UNKNOWN. Field names are documented.
- **close_session** [D-tools, D-cleanup]:
  - `true` on a fleet terminates the streaming instance and triggers autoscaling.
  - On an image builder it does not end the streaming session.
  - `false` keeps the session until the disconnect timeout (fleet) or until the builder is stopped.
- **switch_user / get_current_user** [D-tools]: the return format of get_current_user ("Returns the logged-in built-in user") is UNKNOWN beyond that.

### 5.2 Exact JSON input schemas: status

**AWS publishes no JSON Schema for any tool.** JSON types (integer vs number for x/y, string vs number for duration), `additionalProperties`, descriptions, titles and annotations are all UNDOCUMENTED.

Facts known from the sample about the live `tools/list` [S-readme, S-seq]:

- `agentaccess___execute_sequence.inputSchema.properties.actions.maxItems == 20`.
- `agentaccess___wait.inputSchema.properties.duration.minimum == 0.001` and `.maximum == 5`.
- The other three sequence limits (total wait ≤ 10 s, total type_text ≤ 10,000 chars, 60 s deadline) are "stated only in the execute_sequence description", not in the schema.
- `wait` takes `duration` in **seconds** (fractional allowed) [S-agentsmd].
- Coordinates are integers in all sample calls (e.g. `{"x":500,"y":300}`) [S-agentsmd]. Whether floats are accepted is UNKNOWN.
- Screen space is 1280×720 (the only supported resolution) [D-session, D-api-cfg].

**[TINY-PROPOSAL]** The schemas below are what tinyAgentAccess will publish until someone captures a real AWS `tools/list`. **They are not AWS-verified.** Fields, enums and limits come from §5.1. Types and wording are our guesses.

```json
{"name":"agentaccess___left_click","inputSchema":{"type":"object","properties":{
  "x":{"type":"integer"},"y":{"type":"integer"},
  "modifiers":{"type":"string","description":"e.g. ctrl or ctrl+shift"}},
  "required":["x","y"]}}
```

- `double_click`, `triple_click`, `right_click`, `middle_click`, `left_mouse_down`, `left_mouse_up`: same schema as `left_click`.
- `move_pointer`: `{x:int, y:int}`, both required.
- `left_click_drag`: `{start_x, start_y, end_x, end_y}`, all integers, all required.
- `scroll`: `{x:int, y:int, scroll_direction: enum ["Up","Down","Left","Right"], scroll_amount: integer (ticks, 120 = 1 notch), modifiers?: string}`. Required: `x`, `y`, `scroll_direction`, `scroll_amount`.
- `type_text`: `{text: string, maxLength 10000}`, required.
- `key`: `{keys: string}`, required.
- `hold_key`: `{keys: string, duration: number, min 1, max 30}`, both required.
- `screenshot`: `{include_cursor?: boolean, default false}`.
- `launch_application`: `{id: string}`, required. The id list goes in the description, as AWS does.
- `toggle_app_switcher`, `get_session_info`, `get_current_user`: `{}`.
- `wait`: `{duration: number, minimum 0.001, maximum 5}`, required.
- `execute_sequence`: `{actions: array, maxItems 20, items: object with required "type": string, additionalProperties allowed}`, required.
- `close_session`: `{expire_streaming_session?: boolean, default false}`.
- `switch_user`: `{user: enum ["ADMINISTRATOR","TEMPLATE_USER","TEST_USER"]}`, required.
- Stateless mode: every tool also gets `mcpSessionId: string` (`"NEW"` or an id). Whether it is required is UNKNOWN; we propose required in stateless mode.

### 5.3 Outputs

- **screenshot**: an MCP `image` content block with base64 `data` [S-seq]. The format is PNG or JPEG, following `ScreenImageFormat` [D-session, D-api-cfg]. The sample says "Returns a PNG image… always 1280 × 720" [S-agentsmd]. Each screenshot is about 1–2 MB on the wire [S-pruning]. The exact `mimeType` strings, whether a text block comes with it, and whether `include_cursor` draws the cursor into the image are UNKNOWN. The origin is top-left. That is the standard convention, but AWS does not state it.
- **Action tools** (click, type, key …): the success payload is UNDOCUMENTED. The sample sequence result shows a per-action `status: "success"`.
- **execute_sequence** [D-tools, S-readme]:
  - On success: the server "pauses briefly" and returns a final screenshot. The pause is skipped when the last action is `wait`.
  - On rejection, failure, or deadline: **text only**, with guidance to take a screenshot.
  - Exact shape from the sample [S-readme]. The first content block is JSON text:

```json
{"sequence_summary":{"submitted":7,"executed":7,"succeeded":7,"guidance":""},
 "action_results":[{"index":0,"action":"key","status":"success","duration_ms":2}, "..."],
 "terminal_screenshot":{"status":"success","duration_ms":88}}
```

    - Success is followed by an `image` block.
    - Rejection before dispatch: `isError: true`, every action has `"status":"skipped","reason":"sequence rejected before dispatch"`, and `sequence_summary` gains `stopped_at_index` and `stop_reason`. Example: `"CLIENT_ERROR: Rejected before dispatch: cumulative wait 15.0s exceeds 10.0s"` with `guidance` `"Correct the sequence and retry. No desktop action ran, so no screenshot is needed."`.
    - Documented stop_reason texts: `too many actions: 21 > 20`; `action 0: wait duration must be a number between 0.001 and 5.0 seconds`; `cumulative wait 15.0s exceeds 10.0s`; `cumulative type_text length 11000 exceeds 10000`.
    - Deadline: `isError: true`, text only, e.g. `Deadline reached before action N`. The remaining actions are `skipped`.
    - `action` holds the short name (`key`, `wait`). Each action's `type` must be the name as listed (e.g. `agentaccess___left_click`), with the tool's arguments as sibling keys [D-tools]. The sample says short names are accepted too [S-agentsmd] (SAMPLE-only). `screenshot` and `execute_sequence` cannot be nested [D-tools]. Whether forwarded tools or interface tools can be sequence items is UNKNOWN.
    - The status/reason of a mid-sequence action failure and the error texts: UNKNOWN beyond "stops at first failure".
- **close_session / get_current_user / switch_user** results: UNKNOWN.

### 5.4 Errors

The documented error texts use a category prefix:

- `SERVER_ERROR: Invalid input` (bad launch_application id) [D-tools].
- `CLIENT_ERROR: …` (sequence rejection) [S-readme].
- Tool errors use the MCP result `isError: true` [D-tools].

Other errors seen or documented:

| Condition | Observed form | Source |
|---|---|---|
| Unsupported interface header value | JSON-RPC error `-32602` | D-cui |
| Stateful request without Mcp-Session-Id | HTTP 400 `Missing Mcp-Session-Id header` | D-ident-dj |
| SigV4 failure | HTTP 401 | S-agentsmd |
| Missing IAM permission | HTTP 403 ("Access denied by IAM policy") | S-agentsmd, D-ident-dj |
| No streaming URL / target | `DCV proxy not initialized` | S-agentsmd |
| Desktop not ready | `dcv session not ready` (retryable) | S-agentsmd |
| Transient | `backend unavailable` | S-agentsmd |
| Second agent on same desktop | `CONNECTION_LIMIT_REACHED` / `Another agentic client is already connected` | S-agentsmd |
| Expired or invalid streaming URL | HTTP 400 `… Invalid streaming URL` (e.g. `streaming URL has expired`) | S-agentsmd |
| Expired SAML | HTTP 400 `SAML assertion has expired` | S-readme |
| Session stopped (observer stop, expiry, timeout) | `client_disconnected` | S-retry |
| Tool called before the desktop is attached | `Unknown tool` | S-agentsmd |
| `tools/list` refused (desktop not ready / held) | `tools/list` answered with an `isError` tool *result* instead of a list | S-mcpclient |
| Image builder with an old agent image | `IncompatibleImageException` (AppStream API, not MCP) | D-prereq |
| DJ session faults | `INVALID_AUTH_POLICY`, `CBA certificate fetch failed` | D-ident-dj |

- Exact JSON-RPC codes or HTTP statuses for the SAMPLE rows: UNKNOWN.
- Out-of-range coordinate behavior: the sample says "the service checks" bounds [S-ctool]. The error text is UNKNOWN.

## 6. Feature gating by configuration

- `COMPUTER_VISION` off: screenshot unavailable. `COMPUTER_INPUT` off: mouse/keyboard unavailable [D-session].
- What `tools/list` looks like under each gating combination (tools hidden vs listed-but-refused): UNDOCUMENTED.
- Forwarding needs both the service setting and IAM `CallForwardedTool`. "IAM permissions do not override the service setting" [D-forward].

## 7. Computer-use interfaces (tool dialects)

| Local implementation | Status |
|---|---|
| All five dialects; per-request header on list/call; native adapters | best-effort, unverified vs AWS; all schemas PROVISIONAL vendor-derived, not AWS-confirmed |

Local conversion decisions are recorded in [design.md](design.md#m5-computer-use-dialect-adapters).


Documented [D-cui]:

- Selected by the `x-amzn-agentaccess-computer-use-interface-id` header on `tools/list` **and** every `tools/call`.
- Per request, nothing stored. Omitted or empty means native tools. An unknown value returns **JSON-RPC -32602**.
- The interface substitutes its tools for **standard desktop tools only**; it leaves session, forwarded and application-catalog tools unchanged. The server converts each call into the underlying desktop actions.

| Header value | AWS label |
|---|---|
| (omit) | Default |
| `ANTHROPIC_COMPUTER_20250124` | Anthropic computer use (2025-01-24) |
| `ANTHROPIC_COMPUTER_20251124` | Anthropic computer use (2025-11-24) |
| `ANTHROPIC_COMPUTER_TOOLSET_20260801` | Anthropic computer toolset (2026-08-01) |
| `OPENAI_COMPUTER_V1` | OpenAI computer use v1 |
| `GOOGLE_COMPUTER_USE_3X_DESKTOP_V1` | Gemini computer use 3.x desktop v1 |

**AWS documents no tool schemas for any dialect.** Everything below comes from the sample, which keeps only *name mappings* for logging. It states "Only names are mapped here; the service converts the arguments" [S-cui]. The argument conversions are therefore UNKNOWN unless noted. Per-dialect facts [S-cui, S-ctool, S-readme, S-factory]:

- Detection: every interface replaces `agentaccess___type_text`, so seeing it listed means the interface was not applied. An account where interfaces aren't enabled silently gets the native tools (no error) [S-cui].
- Tools that stay listed alongside interface tools: `agentaccess___wait`, `agentaccess___execute_sequence`, `agentaccess___get_session_info` (plus forwarded and catalog tools) [S-factory, D-cui].
- Interface tool names are **unprefixed**. The sample filters the listed specs named `computer` and the toolset member names exactly [S-model, S-ctool].
- The service "rejects actions the desktop cannot do, such as `wait`, `zoom` and `cursor_position`" [S-readme].

### 7.1 ANTHROPIC_COMPUTER_20251124
- Listed: **one tool `computer`** in the shape of Anthropic's `computer_20251124` [S-readme, S-cui]. The sample sends Claude the typed declaration `{"type":"computer_20251124","name":"computer","display_width_px":1280,"display_height_px":720}` with beta `computer-use-2025-11-24` [S-ctool]. The JSON schema the service lists is UNKNOWN; per the sample it is Anthropic's.
- Action → native mapping (sample naming, inputs per Anthropic's tool) [S-ctool]:

| `action` | native | args |
|---|---|---|
| screenshot | screenshot | — |
| left_click, right_click, middle_click, double_click, triple_click | same name | `coordinate:[x,y]` → x,y; `text` → `modifiers` |
| left_mouse_down, left_mouse_up | same | `coordinate` → x,y; `text` → modifiers |
| mouse_move | move_pointer | `coordinate` → x,y |
| left_click_drag | left_click_drag | `start_coordinate` → start_x/y; `coordinate` → end_x/y |
| scroll | scroll | `coordinate` → x,y; `scroll_direction`, `scroll_amount`; `text` → modifiers |
| type | type_text | `text` → text |
| key | key | `text` → keys |
| hold_key | hold_key | `text` → keys; `duration` |
| wait, zoom, cursor_position | **rejected** by the service; use `agentaccess___wait` | — |

  - **UNKNOWN**: how Anthropic `scroll_amount` (notches) maps to native ticks (×120?) and how direction case maps (`up` → `Up`). The sample passes them through only for logging.
  - Key-name translation (e.g. xdotool `Return`) is also UNKNOWN.

### 7.2 ANTHROPIC_COMPUTER_TOOLSET_20260801
- Listed: **one tool per action** (Anthropic `computer_toolset_20260801`): `screenshot, left_click, right_click, middle_click, double_click, triple_click, left_click_drag, mouse_move, left_mouse_down, left_mouse_up, scroll, type, key, hold_key` [S-ctool].
- **Not listed by the service:** `wait` (the client runs it locally, arg `duration`), `zoom`, `cursor_position`. The sample disables the last two: `{"type":"computer_toolset_20260801","configs":{"zoom":{"enabled":false},"cursor_position":{"enabled":false}}}` [S-ctool].
- The mapping is the same as 7.1, with the tool name in place of `action`. Per-tool input schemas are UNKNOWN.

### 7.3 ANTHROPIC_COMPUTER_20250124
- "Anthropic's older `computer_20250124` tool", offered to the model as plain function tools as listed [S-readme]. Tool name(s), schema and mapping: UNKNOWN. Presumably one `computer` tool, but this is not confirmed.

### 7.4 OPENAI_COMPUTER_V1
- One tool `computer`; the action is named in the field **`type`** [S-cui, S-readme].
- Name mapping [S-cui]:
  - `click` → `left_click` / `right_click` / `middle_click`, chosen by `button` (`left`, the default, / `right` / `middle`).
  - `double_click` → double_click; `drag` → left_click_drag; `move` → move_pointer; `scroll` → scroll.
  - `keypress` → key (a `keys` **list** is joined with `+`); `type` → type_text; `screenshot` → screenshot; `wait` → wait. The sample maps `wait`, but README says the service rejects `wait`, so wait support is UNKNOWN.
- Argument conversion (OpenAI `x`, `y`, `path[]`, `scroll_x/scroll_y`, `keys[]`): UNKNOWN.

### 7.5 GOOGLE_COMPUTER_USE_3X_DESKTOP_V1
- One tool per action; **coordinates on a 0–999 normalized grid** (the server converts to 1280×720) [S-readme].
- Name mapping [S-cui]:
  - `take_screenshot` → screenshot.
  - `click` → left_click; `double_click`, `triple_click`, `middle_click`, `right_click` → same names.
  - `mouse_down` → left_mouse_down; `mouse_up` → left_mouse_up; `move` → move_pointer.
  - `type` → type_text; `press_key` → key; `hotkey` → key (keys list joined with `+`).
  - `scroll` → scroll; `drag_and_drop` → left_click_drag.
- Argument names, the rounding of grid→pixel conversion, and the full tool set: UNKNOWN.

**[TINY-PROPOSAL]** Implement the dialects as adapters over the native action layer. For 7.1/7.2/7.4/7.5, use each vendor's public tool schema. Mark them "best-effort, unverified vs AWS" until real `tools/list` captures are available.

## 8. MCP tool forwarding

Documented [D-forward]:

- Enable `FORWARD_MCP_TOOLS`. The image must contain the config file:
  - Windows: `C:\ProgramData\NICE\dcv\mcp_server_redirection_config.json`
  - Linux: `/etc/dcv/mcp_server_redirection_config.json`
  - Image builders use the same file. Files placed on a running instance don't carry over to new sessions.
- Schema: `{"mcpServers": {"<name>": {"command": "<abs path>", "args": ["..."]}}}`. `command` is required; `args` is optional.
  - **stdio only.** No `env`/`cwd`.
  - Servers inherit the session environment and run as the session user.
  - Use absolute paths; on Windows, forward slashes.
- Naming: `forwarded___{server-name}___{original-tool-name}`. Calls route to the owning server.
- IAM `CallForwardedTool`. Condition keys `StackArn` / `ImageBuilderArn`.

Service contract from the sample's tester [S-fwd-contract, sample tester README]:

- Each `tools/list` and tool call to a forwarded server has a **5 s timeout**. Past it the call is cancelled and the client gets no tools. Spawn + initialize is not bounded.
- Teardown budget 15 s; graceful close 1 s per server.
- The config must be UTF-8 **without BOM** (rejected otherwise). Unknown keys are silently ignored.
- The service does no tool-name validation. A tool name containing `___` makes the forwarded name ambiguous.
- Forwarded tools are not affected by interface headers [D-cui]. In CloudTrail they appear as `CallForwardedTool` (IAM).

## 9. Configuration surface (control plane, AppStream API, not MCP)

`AgentAccessConfig` on CreateStack/UpdateStack/CreateImageBuilder/UpdateImageBuilder [D-api-cfg, D-api-set, D-getstarted]:

| Field | Type / values | Req | Notes |
|---|---|---|---|
| `Settings` | array of `{AgentAction: COMPUTER_VISION\|COMPUTER_INPUT\|FORWARD_MCP_TOOLS, Permission: ENABLED\|DISABLED}` | yes | At least one ENABLED. INPUT requires VISION. Forwarding-only is allowed. |
| `ScreenResolution` | `W_1280xH_720` | yes | only value |
| `ScreenImageFormat` | `PNG`\|`JPEG` | yes | |
| `ScreenshotsUploadEnabled` | bool | no | |
| `S3BucketArn` | S3 ARN pattern | cond. | Required if uploads are enabled. Removed when uploads are set false [D-getstarted]. |
| `UserControlMode` | `VIEW_ONLY`\|`VIEW_STOP`\|`DISABLED` | no | |
| `ReplaySessionEnabled`, `KmsKeyArn` | bool, KMS ARN | no | **Public preview, stacks only** [D-evidence]. **Not in the API_AgentAccessConfig reference page.** |

- Disable with `AttributesToDelete: AGENT_ACCESS_CONFIG`.
- Stack changes apply to new sessions. Image builders must be stopped to change settings [D-getstarted, D-session].
- Screenshot storage:
  - The MCP server uploads **using the agent's credentials**, so the agent needs `s3:PutObject` [D-session].
  - Key: `agentaccess/screenshots/year=YYYY/month=MM/day=DD/{streaming-session-id}/{timestamp}.png` [D-monitor]. The docs show `.png` even for JPEG; behavior with JPEG is UNKNOWN.
- **[TINY-PROPOSAL local mapping, M7]** UserControlMode maps to a same-origin `/observe` page: disabled → 404, `view_only` → watch, `view_stop` → watch + Stop closing the local MCP session through the normal release path. The HTML page itself is unauthenticated; live data and Stop require the observer secret (see [§12](#12-authentication-and-authorization)). A new session is required after Stop. PROVISIONAL: Stop flags the session before waiting for current desktop work; sequences check between steps. AWS mid-action semantics are UNKNOWN. This is local screenshot polling, **not DCV**, with no streaming URL or image-builder stop/start. Keep it on loopback. No client_disconnected notification is emitted because its wire form is UNKNOWN [S-retry]. See [design M7](design.md#m7-local-replay-and-human-control).
- User control mode [D-session]:
  - Observers watch via a streaming URL in a browser. `VIEW_STOP` shows a stop button.
  - On a stack, after a stop the agent must start a new session. On an image builder, agent access returns only after a stop/start.
  - A human opening an image builder with agent access enabled can only watch.
- **[TINY-PROPOSAL local mapping, M7]** `[replay] enabled/save_dir` persists pre-action frames (returned image for screenshot calls) and paired before/after metadata on private local disk. Response is appended after execution because it cannot exist before dispatch (PROVISIONAL). Session closure writes a simple local `index.html`, **not a Bedrock evidence report**. No video, transcript, S3, KMS, cross-Region inference or automatic ≤15-day retention is implemented. Wait/session/forwarded calls are metadata-only; see [local choices and limits](design.md#m7-local-replay-and-human-control).
- Replay Session [D-evidence]:
  - A whole-screen frame plus metadata (tool, arguments, response, timestamp) is captured **just before each action**.
  - After the session, Bedrock generates a video, a transcript and an HTML report, delivered to S3.
  - AWS-side retention is ≤ 15 days. No redaction. Uses cross-Region inference.
- Home folders and app-settings persistence: stacks only. App-settings persistence is Windows-only [D-session].

**[TINY-PROPOSAL]** Model these as a server config file: allowed actions (vision/input/forwarding), screenshot format, optional screenshot sink, observer mode.

## 10. Limits and quotas (behavior-relevant)

- Resolution 1280×720 only. type_text ≤ 10,000 chars. hold_key 1–30 s. wait 0.001–5 s [D-tools].
- execute_sequence: ≤ 20 actions, total wait ≤ 10 s, total text ≤ 10,000 chars, 60 s deadline [D-tools].
- One agent per session, image builder, or named user per fleet [D-endpoint, D-prereq].
- Not supported [D-prereq]: VPC endpoints, multi-session fleets, Elastic fleets, domain-joined image builders, app block builders, Linux image builders.
- **Correction vs earlier draft:** "an AI agent or a human can provide input, not both" is documented **for image builders only** [D-prereq].
- Supported OSes: Windows Server, Linux (AWS base images), Windows 11 (BYOL) [D-prereq].
- Image builders need an agent image released on or after Oct 25, 2026 (base image on or after Nov 4, 2026) [D-prereq].
- Request rate limits / throttling: UNDOCUMENTED.
- Pricing "scales based on active session time" [W-ga]. Not behavior-relevant.

## 11. Underlying AWS APIs

- Called by the **client**, not the MCP server:
  - `appstream:CreateStreamingURL`, `appstream:CreateImageBuilderStreamingURL`, `appstream:DescribeFleets` (prereq policy) [D-prereq].
  - Config APIs: `CreateStack`/`UpdateStack`/`CreateImageBuilder`/`UpdateImageBuilder`/`DescribeImageBuilders`/`DescribeImages` [D-getstarted, D-prereq].
- Implied **server-side** actions:
  - Creating a streaming session from stack/user headers (it says "the MCP server creates the streaming session for you").
  - Expiring streaming sessions (close/expire).
  - S3 PutObject with the agent's credentials.
  - The appstream:Stream check for DJ (`INVALID_AUTH_POLICY`) [D-endpoint, D-session, D-ident-dj].
  - Which internal APIs it calls: UNDOCUMENTED.

## 12. Authentication and authorization

### AWS AgentAccess documentation and sample behavior

- **SigV4**: service name `agentaccess-mcp`, region = endpoint region, sign every request [D-endpoint]. The SAML `_meta` must be in the body before signing (the signature covers the payload) [D-ident-dj, S-djauth].
- **OAuth 2.1 via AWS Sign-In** (same as the AWS MCP Server) [D-endpoint]:
  - Bearer tokens.
  - Interactive (dynamic client registration + browser) or non-interactive (client-credentials exchange of SigV4 creds).
  - `login_hint` query param = URL-encoded IdP sign-in URL.
  - `SourceIdentity` SAML attribute → streaming user.
- **mcp-proxy-for-aws** [G-proxy, D-endpoint]:
  - `aws_iam_streamablehttp_client(endpoint, aws_service, aws_region, headers, metadata, timeout, credentials)`.
  - `metadata` needs ≥ 1.6.1 and injects `_meta`.
  - CLI: `uvx mcp-proxy-for-aws <endpoint> --service agentaccess-mcp --region <r> [--profile] [--metadata K=V]` [S-agentsmd].
  - The sample warns that adding `_meta` after signing "cannot work with SigV4" [S-agentsmd]. This contradicts the docs' recommendation; unresolved.
- **IAM** [D-sar]: 25 actions:
  - InvokeMcp (initialize + tools/list, Write).
  - GetScreenshot, CheckConnectionStatus, GetSessionInfo, GetCurrentUser (Read).
  - LeftClick, DoubleClick, TripleClick, RightClick, MiddleClick, LeftClickDrag, LeftMouseDown, LeftMouseUp, MovePointer, Scroll, TypeText, KeyPress, HoldKey, LaunchApplication, ToggleAppSwitcher, ExecuteSequence, Wait, CloseSession, SwitchUser, CallForwardedTool (Write).
  - No resource ARNs. Condition keys: `agentaccess-mcp:StackArn` (ARN), `agentaccess-mcp:ImageBuilderArn` (ARN), `agentaccess-mcp:LocalUser` (String; present only on log-in requests, so use `StringEqualsIfExists`) [D-prereq].
  - The prereq page's action list omits GetSessionInfo, CloseSession, ExecuteSequence, Wait [D-prereq vs D-sar].
  - How sequence sub-actions are authorized (ExecuteSequence alone, or each sub-action's action too) is UNDOCUMENTED.
- **Agent identity** (OBO → AssumeRoleWithWebIdentity) [D-ident]:
  - Session tags `AgentAccessAgentId`, `AgentAccessAgentName` (optional), `AgentAccessUserId`.
  - Source identity = streaming user.
- **Domain-joined fleets**: SAML assertion + stack ARN in `_meta` (§3). Certificate-based auth is required. The stack must be in the signer's account [D-endpoint, D-ident-dj].

### [TINY-PROPOSAL] Shipped local authentication

tinyAgentAccess deliberately departs from the managed IAM/SigV4 and OAuth 2.1 via AWS Sign-In mechanisms documented above [D-sar, D-endpoint]. Local authentication is kept independent of those AWS services, which cannot map to a self-hosted binary with no AWS account, IAM, or AWS Sign-In. The local scheme provides no AWS authentication protocol parity. Local OAuth 2.1 support is deferred.

- **Agent requests.** Every `/mcp` request (POST, GET, and DELETE, including `initialize`) requires exactly one `Authorization: Bearer <token>` header. The server checks it before reading the body and compares tokens in constant time. After Host/Origin checks pass, a missing, incorrect, or duplicate header returns 401 without `WWW-Authenticate` or `resource_metadata`. Those browser checks return 403 first. There is no production no-auth mode.
- **Token file.** The long-lived agent secret defaults to `tinyagentaccess/mcp-token` under the user state directory. New files are created with mode `0600` on Unix and an owner-only ACL on Windows. Existing Unix files are rejected if any group or other permission bit is set. Existing Windows files must be owned by the current user; their allow ACEs may also include SYSTEM, Administrators, and OWNER RIGHTS (`OWNER RIGHTS` is trusted only after the owner is the current user). A path set by `--token-file` or `[auth] token_file` must already exist, so a typo fails instead of creating a secret. Startup fails closed if a required credential cannot be loaded or fails validation. `rotate-token` replaces only the agent token.
- **Remote access.** The default listener binds to loopback. The documented remote path is an SSH tunnel to that listener. Wildcard binds are rejected. A concrete non-loopback bind requires both the token and explicit opt-in with `--allow-non-loopback` or `[auth] allow_non_loopback = true`. The server still warns that it has no TLS.
- **Observer.** The `/observe` live APIs and Stop require a separate secret stored in `observe-token`. The HTML page itself does not require the secret. Browsers omit URL fragments from requests, so the page reads the secret from `/observe#<secret>` and sends it in `X-TAA-Observe-Token`. Neither secret grants access to the other's APIs. Stop does not lock out the agent or rotate its token. Host/Origin checks still apply independently of authentication.
- **Diagnostics.** `tinyagentaccess doctor` reports the default agent token path, existence, and access status in text and in additive JSON fields. `--token-file` and `[auth] token_file` override that path at `serve`; `doctor` does not inspect the override. It never prints or creates a token.
- **Residual risk.** Other processes running as the same user can read the token file. Forwarded children can still read the `0600` file even though the launcher strips `TAA_TOKEN` from their environments.

## 13. Observability (for parity, optional in tiny)

- CloudTrail, event source `agentaccess-mcp.amazonaws.com` [D-ident-ndj]:
  - Management event `Initialize`.
  - Tool calls are **data events** named `CallReadOnlyTool` / `CallPrivilegedTool` [D-ident-ndj, D-ident-dj].
  - The Monitoring page instead says "one for each agentaccess-mcp action (for example GetScreenshot…)" [D-monitor].
  - **Correction vs earlier draft:** the event names are documented; the docs just describe them two ways.
- CloudWatch metrics: Invocations, Latency, ClientErrors, ServerErrors, McpSessionStart, McpSessionDuration [D-monitor]. Namespace and dimensions: UNDOCUMENTED.

## 14. Client setup examples (verbatim patterns)

- Python SigV4 via mcp-proxy-for-aws with `headers={"x-amzn-agentaccess-streaming-session-url": url}` [D-endpoint, D-getstarted].
- Image builder: add `"x-amzn-agentaccess-local-user": "TEST_USER"` and `timeout=120` [D-endpoint].
- POLLING loop [D-connmode]. Raw JSON-RPC example (initialize → tools/list → `tools/call agentaccess___screenshot {}`) [D-ident-dj].
- IDE stdio config [S-agentsmd]: `{"type":"stdio","command":"uvx","args":["mcp-proxy-for-aws","https://agentaccess-mcp.us-east-1.api.aws/mcp","--service","agentaccess-mcp","--region","us-east-1"]}`.
- Bedrock AgentCore: a remote MCP server tool with stack/user headers and IAM or OAuth outbound auth [D-agentcore].
- Supported clients and frameworks: Claude Code, Kiro, Cursor, Codex, LangChain, CrewAI, Strands, AgentCore, LangGraph Platform [D-llms].

## 15. Biggest unknowns (capture from a live AWS account to close)

1. Exact `tools/list` JSON: input schemas, types, descriptions, annotations, `outputSchema`/`structuredContent` use. **This is the #1 gap.** Only `actions.maxItems=20` and `wait.duration` min/max are known.
2. Success payloads of action tools, get_current_user, switch_user, close_session. The get_session_info encoding. Whether `_meta.streaming_url` is returned.
3. `connection_status`: listed name/prefix, all `state` values, other fields.
4. All five dialect schemas and the argument conversions: scroll units, key names, 0–999 grid rounding, OpenAI arg names, and the whole of ANTHROPIC_COMPUTER_20250124.
5. Error envelope: which failures are HTTP vs JSON-RPC vs `isError`, and their codes. The meaning of -32022.
6. Key-name vocabulary for `key`/`hold_key`/`modifiers`.
7. Stateless `mcpSessionId` as a schema parameter (documented only in the sample).
8. Fleet MCP-session idle timeout. Throttling. Resources/prompts (assumed none).
9. Whether tools hidden by COMPUTER_VISION/INPUT settings are unlisted or refused.
10. Screenshot `mimeType` strings, cursor rendering, the S3 key extension for JPEG.

## 16. Sources

AWS docs (developer guide base `https://docs.aws.amazon.com/appstream2/latest/developerguide/`):
- D-agent-access: …/agent-access.html
- D-learn-more: …/agent-access-learn-more.html
- D-concepts: …/agent-access-concepts.html
- D-llms: …/agent-access-agents-llms.html
- D-bp: …/agent-access-best-practices.html
- D-mcp-server: …/agent-access-mcp-server.html
- D-overview: …/agent-access-mcp-server-overview.html
- D-prereq: …/agent-access-mcp-prerequisites.html
- D-session: …/agent-access-mcp-session-behavior.html
- D-endpoint: …/agent-access-mcp-server-endpoint.html
- D-ident: …/agent-access-agent-identity.html
- D-ident-ndj: …/agent-access-agent-identity-ndj.html
- D-ident-dj: …/agent-access-agent-identity-dj.html
- D-connmode: …/agent-access-mcp-server-connect-mode.html
- D-cleanup: …/agent-access-mcp-server-session-cleanup.html
- D-tools: …/agent-access-mcp-server-tools.html
- D-cui: …/agent-access-computer-use-interfaces.html
- D-forward: …/agent-access-mcp-tool-forwarding-details.html
- D-monitor: …/agent-access-mcp-server-monitoring.html
- D-evidence: …/agent-access-session-evidence.html
- D-agentcore: …/agent-access-agentcore.html
- D-getstarted: …/getting-started-agent-access.html
- D-regions: …/allowed-domains.html#allowed-domains-mcp-endpoints
- D-api-cfg: https://docs.aws.amazon.com/appstream2/latest/APIReference/API_AgentAccessConfig.html
- D-api-set: https://docs.aws.amazon.com/appstream2/latest/APIReference/API_AgentAccessSetting.html
- D-api-csu: https://docs.aws.amazon.com/appstream2/latest/APIReference/API_CreateStreamingURL.html
- D-api-cibsu: https://docs.aws.amazon.com/appstream2/latest/APIReference/API_CreateImageBuilderStreamingURL.html
- D-sar: https://docs.aws.amazon.com/service-authorization/latest/reference/list_agentaccess-mcp.html
- D-reg: https://docs.aws.amazon.com/online-register/latest/data-formats/amazonworkspacesagentaccessmcpserver.html

AWS announcements:
- W-ga: https://aws.amazon.com/about-aws/whats-new/2026/06/amazon-workspaces-ai/ (Jun 30, 2026)
- W-oct: https://aws.amazon.com/about-aws/whats-new/2026/10/workspaces-ai-agents-oauth-connection-management/ (Oct 20, 2026)
- W-nov: https://aws.amazon.com/about-aws/whats-new/2026/11/workspaces-ai-agents-linux-windows-11/ (Nov 3, 2026)
- P-product: https://aws.amazon.com/workspaces/ai-agents/

AWS GitHub, sample repo `https://github.com/aws-samples/sample-code-for-workspaces-agent-access` @ `19839c7`:
- S-readme: README.md
- S-agentsmd: AGENTS.md
- S-cui: lib/computer_use_interface.py
- S-ctool: lib/computer_tool.py
- S-model: lib/model.py
- S-factory: lib/agent_factory.py
- S-mcpclient: lib/mcp_client.py
- S-djauth: lib/domain_join_auth.py
- S-retry: lib/retry.py
- S-pruning: lib/screenshot_pruning_manager.py
- S-seq: tool_sequencing.py
- S-quick: quickstart.py
- S-fwd-contract: utils/mcpforwardingtester/src/mcp_forwarding_tester/contract.py

Other AWS GitHub:
- G-proxy: https://github.com/aws/mcp-proxy-for-aws (README: `--metadata` / `metadata` injects `_meta`)
- No agentaccess server code was found in awslabs/mcp. The GitHub code-search API was rate-limited, so this check is not exhaustive.

The pages were read on Nov 16, 2026; the URLs above are the canonical sources. The sample repository was read at commit `19839c7`.
