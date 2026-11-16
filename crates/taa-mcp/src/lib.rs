//! Streamable HTTP, without TLS or an MCP framework dependency.
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use taa_platform::Platform;
use taa_tools::{client, config::Config, result, text, PREFIX};
pub const MODERN: &str = "2026-07-28";
pub const STATEFUL: [&str; 3] = ["2025-03-26", "2025-06-18", "2025-11-25"];
pub use taa_tools::dialects::Interface;
struct Session {
    version: String,
    touched: Instant,
    active: usize,
    forwarding: Option<Arc<taa_forward::ForwardingSet>>,
}
#[derive(Default)]
struct Sessions {
    open: HashMap<String, Session>,
    closed: HashMap<String, Instant>,
}
pub struct Server {
    pub config: Config,
    pub platform: Arc<dyn Platform>,
    sessions: Mutex<Sessions>,
    desktop: Arc<tokio::sync::Mutex<()>>,
    held: Arc<Mutex<Option<Held>>>,
}
impl Server {
    pub fn new(config: Config, platform: Arc<dyn Platform>) -> Arc<Self> {
        let server = Arc::new(Self {
            config,
            platform,
            sessions: Mutex::new(Sessions::default()),
            desktop: Arc::default(),
            held: Arc::default(),
        });
        // PROVISIONAL (not published by AWS): poll idle expiry once per second.
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let weak = Arc::downgrade(&server);
            runtime.spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_secs(1));
                loop {
                    tick.tick().await;
                    let Some(server) = weak.upgrade() else {
                        break;
                    };
                    server.cleanup(&mut server.sessions.lock().unwrap());
                }
            });
        }
        server
    }
    fn cleanup(&self, s: &mut Sessions) {
        s.open.retain(|id, v| {
            let keep = v.active > 0
                || v.touched.elapsed()
                    < Duration::from_secs(self.config.sessions.idle_timeout_secs);
            if !keep {
                teardown_later(v.forwarding.clone());
                let desktop = self.desktop.clone();
                let held = self.held.clone();
                let platform = self.platform.clone();
                let id = id.clone();
                tokio::spawn(async move {
                    let _desktop = desktop.lock().await;
                    let _ =
                        tokio::task::spawn_blocking(move || release_owned(&platform, &held, &id))
                            .await;
                });
            }
            keep
        });
        s.closed
            .retain(|_, at| at.elapsed() < Duration::from_secs(300));
    }
    fn open(&self, version: &str) -> Result<String, String> {
        let mut s = self.sessions.lock().unwrap();
        self.cleanup(&mut s);
        if s.open.len() >= self.config.sessions.max_sessions {
            return Err(
                "CONNECTION_LIMIT_REACHED: Another agentic client is already connected".into(),
            );
        }
        let id = uuid::Uuid::new_v4().to_string();
        s.open.insert(
            id.clone(),
            Session {
                version: version.into(),
                touched: Instant::now(),
                active: 0,
                // PROVISIONAL (not published by AWS): lazy, isolated per-session environment.
                forwarding: self.config.agent_access.forward_mcp_tools.then(|| {
                    taa_forward::ForwardingSet::new(
                        self.config.forwarding.path(),
                        Default::default(),
                    )
                }),
            },
        );
        Ok(id)
    }
    fn touch(&self, id: &str) -> Option<String> {
        let mut s = self.sessions.lock().unwrap();
        self.cleanup(&mut s);
        let v = s.open.get_mut(id)?;
        v.touched = Instant::now();
        Some(v.version.clone())
    }
    async fn release(&self, id: &str) {
        let held = self.held.clone();
        let platform = self.platform.clone();
        let id = id.to_owned();
        let _ = tokio::task::spawn_blocking(move || release_owned(&platform, &held, &id)).await;
    }

    fn forwarding(&self, id: &str) -> Option<Arc<taa_forward::ForwardingSet>> {
        self.sessions
            .lock()
            .unwrap()
            .open
            .get(id)
            .and_then(|v| v.forwarding.clone())
    }
    pub async fn shutdown(&self) {
        let sets: Vec<_> = self
            .sessions
            .lock()
            .unwrap()
            .open
            .drain()
            .filter_map(|(_, s)| s.forwarding)
            .collect();
        let mut tasks = tokio::task::JoinSet::new();
        for set in sets {
            tasks.spawn(async move { set.teardown().await });
        }
        while tasks.join_next().await.is_some() {}
    }
    fn close(&self, id: &str) {
        let mut s = self.sessions.lock().unwrap();
        if let Some(session) = s.open.remove(id) {
            teardown_later(session.forwarding);
        }
        s.closed.insert(id.into(), Instant::now());
    }
}
// PROVISIONAL (not published by AWS): teardown runs off the HTTP response path.
fn teardown_later(set: Option<Arc<taa_forward::ForwardingSet>>) {
    if let Some(set) = set {
        tokio::spawn(async move {
            set.teardown().await;
        });
    }
}
// Operation::Button and Drag always control the left button in the shared input backend.
// Hold-key releases synchronously; only left button-down persists.
struct Held {
    owner: String,
    point: taa_platform::Point,
}
// Called only while holding the desktop lock. The mutex also keeps dispatch and
// tracking together if the HTTP future is cancelled while its blocking worker runs.
fn release_owned(platform: &Arc<dyn Platform>, held: &Mutex<Option<Held>>, id: &str) {
    let mut held = held.lock().unwrap();
    if held.as_ref().is_some_and(|h| h.owner == id) {
        match platform.perform(taa_platform::Operation::ReleaseLeft) {
            Ok(()) => *held = None,
            Err(e) => eprintln!("warning: session input release failed: {e}"),
        }
    }
}
struct SessionPlatform {
    inner: Arc<dyn Platform>,
    held: Arc<Mutex<Option<Held>>>,
    owner: String,
}
impl Platform for SessionPlatform {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn screen_size(&self) -> (u32, u32) {
        self.inner.screen_size()
    }
    fn capture(&self) -> Result<taa_platform::Frame, String> {
        self.inner.capture()
    }
    fn capture_geometry(&self) -> Result<(u32, u32), String> {
        self.inner.capture_geometry()
    }
    fn perform(&self, op: taa_platform::Operation) -> Result<(), String> {
        use taa_platform::{Button, Operation};
        let mut held = self.held.lock().unwrap();
        // A partial native failure may already have pressed the button.
        if let Operation::Button(point, true, _) = &op {
            *held = Some(Held {
                owner: self.owner.clone(),
                point: *point,
            });
        }
        let result = self.inner.perform(op.clone());
        if result.is_ok() {
            match op {
                Operation::ReleaseLeft
                | Operation::Button(_, false, _)
                | Operation::Drag(_, _)
                | Operation::Click(_, Button::Left, _, _) => *held = None,
                Operation::Move(point)
                | Operation::Click(point, _, _, _)
                | Operation::Button(point, true, _)
                | Operation::Scroll(point, _, _, _) => {
                    if let Some(h) = held.as_mut() {
                        h.point = point;
                    }
                }
                _ => (),
            }
        }
        result
    }
}
// Drop also runs if the HTTP task is cancelled.
struct ActiveSession(Arc<Server>, String);
impl Drop for ActiveSession {
    fn drop(&mut self) {
        if let Some(v) = self.0.sessions.lock().unwrap().open.get_mut(&self.1) {
            v.active -= 1;
            v.touched = Instant::now();
        }
    }
}
pub const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
pub fn router(server: Arc<Server>) -> Router {
    Router::new()
        .route("/mcp", post(handle).get(get).delete(delete))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(server)
}
fn header<'a>(h: &'a HeaderMap, k: &str) -> &'a str {
    h.get(k).and_then(|v| v.to_str().ok()).unwrap_or("")
}
fn local_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}
fn authority_host(a: &str) -> &str {
    let Ok(parsed) = a.parse::<axum::http::uri::Authority>() else {
        return "";
    };
    if parsed.port().is_some() && parsed.port_u16().is_none() {
        return "";
    }
    if let Some(s) = a.strip_prefix('[') {
        s.split(']').next().unwrap_or("")
    } else {
        a.split(':').next().unwrap_or("")
    }
}
fn security(h: &HeaderMap, c: &Config) -> bool {
    if h.get_all("host").iter().count() != 1 || h.get_all("origin").iter().count() > 1 {
        return false;
    }
    let host = authority_host(header(h, "host"));
    let bind = c.bind.parse::<std::net::SocketAddr>().ok();
    if !local_host(host)
        && !bind.is_some_and(|b| !b.ip().is_loopback() && host == b.ip().to_string())
    {
        return false;
    }
    if h.contains_key("origin") {
        let o = header(h, "origin");
        let Some(rest) = o
            .strip_prefix("http://")
            .or_else(|| o.strip_prefix("https://"))
        else {
            return false;
        };
        if rest.contains(['@', '/', '?', '#']) || !local_host(authority_host(rest)) {
            return false;
        }
    }
    true
}
async fn get(State(s): State<Arc<Server>>, h: HeaderMap) -> Response {
    if !security(&h, &s.config) {
        return StatusCode::FORBIDDEN.into_response();
    }
    StatusCode::METHOD_NOT_ALLOWED.into_response()
}
async fn delete(State(s): State<Arc<Server>>, h: HeaderMap) -> Response {
    if !security(&h, &s.config) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if header(&h, "mcp-protocol-version") == MODERN {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let id = header(&h, "mcp-session-id");
    if id.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing Mcp-Session-Id header").into_response();
    }
    let actual_version = match s.touch(id) {
        None => return StatusCode::NOT_FOUND.into_response(),
        Some(v) if v == MODERN => return StatusCode::METHOD_NOT_ALLOWED.into_response(),
        Some(v) => v,
    };
    let version = header(&h, "mcp-protocol-version");
    if !version.is_empty() && !STATEFUL.contains(&version) {
        return (StatusCode::BAD_REQUEST, "Unsupported MCP-Protocol-Version").into_response();
    }
    if !version.is_empty() && version != actual_version {
        return (
            StatusCode::BAD_REQUEST,
            "Protocol version does not match session",
        )
            .into_response();
    }
    let _desktop = s.desktop.lock().await;
    s.release(id).await;
    s.close(id);
    StatusCode::NO_CONTENT.into_response()
}
fn rpc(id: &Value, r: Value) -> Response {
    Json(json!({"jsonrpc":"2.0","id":id,"result":r})).into_response()
}
fn error(id: &Value, code: i64, message: &str, session: Option<&str>) -> Response {
    let mut e = json!({"code":code,"message":message});
    if let Some(s) = session {
        e["data"] = json!({"mcpSessionId":s});
    }
    let status = if matches!(code, -32700 | -32600) {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    };
    (status, Json(json!({"jsonrpc":"2.0","id":id,"error":e}))).into_response()
}
fn modern_result(id: &Value, mut r: Value, session: &str) -> Response {
    if let Some(content) = r["content"].as_array_mut() {
        content.push(text(json!({"mcpSessionId":session}).to_string()));
    }
    r["mcpSessionId"] = session.into();
    r["resultType"] = "complete".into();
    rpc(id, r)
}
fn args_session(p: &Value) -> Option<&str> {
    p["arguments"]["mcpSessionId"]
        .as_str()
        .filter(|s| *s != "NEW")
}
fn request_error(
    modern: bool,
    id: &Value,
    code: i64,
    message: &str,
    session: Option<&str>,
) -> Response {
    let mut response = error(id, code, message, session);
    if modern {
        *response.status_mut() = match code {
            -32700 | -32600 | -32602 | -32020 | -32022 => StatusCode::BAD_REQUEST,
            -32601 => StatusCode::NOT_FOUND,
            _ => StatusCode::OK,
        };
    }
    response
}
async fn handle(State(s): State<Arc<Server>>, h: HeaderMap, body: Bytes) -> Response {
    if !security(&h, &s.config) {
        return StatusCode::FORBIDDEN.into_response();
    }
    for name in [
        "mcp-protocol-version",
        "mcp-session-id",
        "mcp-method",
        "mcp-name",
        "content-type",
    ] {
        if h.get_all(name).iter().count() > 1 || h.get(name).is_some_and(|v| v.to_str().is_err()) {
            return if name == "content-type" {
                StatusCode::BAD_REQUEST.into_response()
            } else {
                request_error(
                    true,
                    &Value::Null,
                    -32020,
                    "Invalid or duplicate MCP routing header",
                    None,
                )
            };
        }
    }
    if !header(&h, "content-type")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .eq_ignore_ascii_case("application/json")
    {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    }
    let accepts_json = h
        .get_all("accept")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|v| {
            matches!(
                v.split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase()
                    .as_str(),
                "application/json" | "application/*" | "*/*"
            )
        });
    if !accepts_json {
        return StatusCode::NOT_ACCEPTABLE.into_response();
    }
    let v: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return error(&Value::Null, -32700, "Parse error", None),
    };
    if let Some(batch) = v.as_array() {
        // [DOC] 2025-03-26 requires receiving batches; later revisions removed them.
        let sid = header(&h, "mcp-session-id");
        if batch.is_empty()
            || s.touch(sid).as_deref() != Some(STATEFUL[0])
            || batch.iter().any(|v| v["method"] == "initialize")
        {
            return error(
                &Value::Null,
                -32600,
                "Invalid Request (batch unavailable for this protocol)",
                None,
            );
        }
        let mut replies = Vec::new();
        for item in batch {
            let response = if item.is_object() {
                handle_one(s.clone(), h.clone(), item.clone()).await
            } else {
                error(&Value::Null, -32600, "Invalid batch member", None)
            };
            if response.status() == StatusCode::ACCEPTED {
                continue;
            }
            let data = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap_or_default();
            match serde_json::from_slice::<Value>(&data) {
                Ok(reply) => replies.push(reply),
                Err(_) => return (StatusCode::BAD_REQUEST, "Invalid batch member").into_response(),
            }
        }
        return if replies.is_empty() {
            StatusCode::ACCEPTED.into_response()
        } else {
            Json(replies).into_response()
        };
    }
    handle_one(s, h, v).await
}
async fn handle_one(s: Arc<Server>, h: HeaderMap, v: Value) -> Response {
    let modern = (!header(&h, "mcp-protocol-version").is_empty()
        && !STATEFUL.contains(&header(&h, "mcp-protocol-version")))
        || v["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"] == MODERN;
    let id = v.get("id").cloned().unwrap_or(Value::Null);
    if !v.is_object()
        || v["jsonrpc"] != "2.0"
        || !v["method"].is_string()
        || v.get("id")
            .is_some_and(|i| !i.is_string() && !i.is_i64() && !i.is_u64())
    {
        return request_error(modern, &Value::Null, -32600, "Invalid Request", None);
    }
    let method = v["method"].as_str().unwrap();
    if method == "initialize" && v.get("id").is_none() {
        return StatusCode::ACCEPTED.into_response();
    }
    let params = v.get("params").cloned().unwrap_or(json!({}));
    if !params.is_object() {
        let mut response = request_error(modern, &id, -32602, "params must be an object", None);
        *response.status_mut() = StatusCode::BAD_REQUEST;
        return response;
    }
    let hv = header(&h, "mcp-protocol-version");
    let mv = params["_meta"]["io.modelcontextprotocol/protocolVersion"]
        .as_str()
        .unwrap_or("");
    if v.get("id").is_some() && !hv.is_empty() && !mv.is_empty() && hv != mv {
        return request_error(
            modern,
            &id,
            -32020,
            "Protocol version header and metadata disagree",
            args_session(&params),
        );
    }
    let version = if hv.is_empty() { mv } else { hv };
    let modern = version == MODERN;
    if (!version.is_empty() && !modern && !STATEFUL.contains(&version))
        || (method == "server/discover" && !modern)
    {
        let mut data = json!({"supported":[MODERN],"requested":version});
        if let Some(sid) = args_session(&params) {
            data["mcpSessionId"] = json!(sid);
        }
        return (StatusCode::BAD_REQUEST, Json(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32022,"message":"Unsupported protocol version","data":data}}))).into_response();
    }
    if modern && v.get("id").is_none() {
        return StatusCode::ACCEPTED.into_response();
    }
    if modern
        && (!params["_meta"].is_object()
            || mv.is_empty()
            || !params["_meta"]["io.modelcontextprotocol/clientCapabilities"].is_object())
    {
        return request_error(
            modern,
            &id,
            -32602,
            "Missing required per-request protocol metadata",
            args_session(&params),
        );
    }
    if modern
        && (header(&h, "mcp-method") != method
            || hv != mv
            || (method == "tools/call"
                && header(&h, "mcp-name") != params["name"].as_str().unwrap_or("")))
    {
        return request_error(
            modern,
            &id,
            -32020,
            "MCP routing headers do not match request",
            args_session(&params),
        );
    }
    if method == "server/discover" {
        return rpc(
            &id,
            json!({"supportedVersions":[MODERN],"ttlMs":0,"cacheScope":"private","capabilities":{"tools":{"listChanged":false}},"resultType":"complete","_meta":{"io.modelcontextprotocol/serverInfo":{"name":"tinyAgentAccess","version":env!("CARGO_PKG_VERSION")}}}),
        );
    }
    if method == "initialize" {
        if modern {
            return request_error(
                modern,
                &id,
                -32601,
                "initialize is not available in stateless protocol",
                None,
            );
        }
        if !params["protocolVersion"].is_string()
            || !params["capabilities"].is_object()
            || !params["clientInfo"].is_object()
        {
            return request_error(
                modern,
                &id,
                -32602,
                "initialize requires protocolVersion, capabilities, and clientInfo",
                None,
            );
        }
        let requested = params["protocolVersion"].as_str().unwrap_or("");
        let chosen = if STATEFUL.contains(&requested) {
            requested
        } else {
            STATEFUL[2]
        };
        let sid = match s.open(chosen) {
            Ok(id) => id,
            Err(e) => return request_error(modern, &id, -32000, &e, None),
        };
        let mut response = rpc(
            &id,
            json!({"protocolVersion":chosen,"capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"tinyAgentAccess","version":env!("CARGO_PKG_VERSION")}}),
        );
        response
            .headers_mut()
            .insert("mcp-session-id", sid.parse().unwrap());
        return response;
    }
    let transport_id = header(&h, "mcp-session-id");
    let actual_version = if modern {
        MODERN.to_string()
    } else {
        if transport_id.is_empty() {
            return (StatusCode::BAD_REQUEST, "Missing Mcp-Session-Id header").into_response();
        }
        match s.touch(transport_id) {
            Some(v) if v != MODERN => v,
            _ => return StatusCode::NOT_FOUND.into_response(),
        }
    };
    if !modern && !version.is_empty() && version != actual_version {
        return (
            StatusCode::BAD_REQUEST,
            "Protocol version does not match session",
        )
            .into_response();
    }
    if v.get("id").is_none() {
        return StatusCode::ACCEPTED.into_response();
    }
    if method == "ping" {
        return rpc(&id, json!({}));
    }
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let existing = if modern {
        args["mcpSessionId"].as_str().filter(|i| *i != "NEW")
    } else {
        None
    };
    let forwarded_call = method == "tools/call"
        && params["name"]
            .as_str()
            .is_some_and(|name| name.starts_with("forwarded___"));
    let interface = if matches!(method, "tools/list" | "tools/call") && !forwarded_call {
        match Interface::parse(header(&h, "x-amzn-agentaccess-computer-use-interface-id")) {
            Ok(i) => i,
            Err(e) => return request_error(modern, &id, -32602, &e, existing),
        }
    } else {
        None
    };
    match method {
        "tools/list" => {
            // Keep session-backed discovery alive while a slow initialize is pending.
            let discovery_id = if modern {
                existing.unwrap_or("")
            } else {
                transport_id
            };
            let _active_discovery = {
                let mut sessions = s.sessions.lock().unwrap();
                sessions.open.get_mut(discovery_id).map(|session| {
                    session.active += 1;
                    session.touched = Instant::now();
                    ActiveSession(s.clone(), discovery_id.to_owned())
                })
            };
            let mut r = taa_tools::dialects::list(interface, &s.config, modern);
            // §8: forwarded tools remain visible regardless of the interface header.
            // PROVISIONAL (not published by AWS): modern discovery uses a short-lived
            // environment when no mcpSessionId is supplied (list does not open sessions).
            let attached = if modern {
                existing.and_then(|id| s.forwarding(id))
            } else {
                s.forwarding(transport_id)
            };
            let temporary = modern && attached.is_none() && s.config.agent_access.forward_mcp_tools;
            let set = attached.or_else(|| {
                temporary.then(|| {
                    taa_forward::ForwardingSet::new(s.config.forwarding.path(), Default::default())
                })
            });
            if let Some(set) = set {
                let mut forwarded = set.list().await;
                // PROVISIONAL (not published by AWS): modern session schema injection;
                // preserve every other downstream tool field.
                if modern {
                    for tool in &mut forwarded {
                        let schema = &mut tool["inputSchema"];
                        if !schema["properties"].is_object() {
                            schema["properties"] = json!({});
                        }
                        schema["properties"]["mcpSessionId"] =
                            taa_tools::schema("wait", true)["properties"]["mcpSessionId"].clone();
                        if !schema["required"].is_array() {
                            schema["required"] = json!([]);
                        }
                        if !schema["required"]
                            .as_array()
                            .unwrap()
                            .contains(&json!("mcpSessionId"))
                        {
                            schema["required"]
                                .as_array_mut()
                                .unwrap()
                                .push(json!("mcpSessionId"));
                        }
                    }
                }
                r["tools"].as_array_mut().unwrap().extend(forwarded);
                if temporary {
                    teardown_later(Some(set));
                }
            }
            if modern {
                r["resultType"] = "complete".into();
                r["ttlMs"] = json!(0);
                r["cacheScope"] = json!("private");
            }
            rpc(&id, r)
        }
        "tools/call" => {
            let Some(name) = params["name"].as_str() else {
                return request_error(modern, &id, -32602, "name must be a string", existing);
            };
            if !args.is_object()
                && name.starts_with("forwarded___")
                && s.config.agent_access.forward_mcp_tools
            {
                let failure = client("arguments must be an object");
                return if modern {
                    modern_result(&id, failure, "")
                } else {
                    rpc(&id, failure)
                };
            }
            let n = name.strip_prefix(PREFIX).unwrap_or("");
            let sid = if modern {
                let Some(requested) = args["mcpSessionId"].as_str() else {
                    return modern_result(&id, client("mcpSessionId is required"), "");
                };
                if requested == "NEW" {
                    match s.open(MODERN) {
                        Ok(i) => i,
                        Err(e) => return modern_result(&id, client(e), "NEW"),
                    }
                } else if s.touch(requested).as_deref() == Some(MODERN) {
                    requested.into()
                } else if n == "close_session"
                    && s.sessions.lock().unwrap().closed.contains_key(requested)
                {
                    return modern_result(&id, result("Session closed", false), requested);
                } else {
                    return modern_result(&id, client("Unknown mcpSessionId"), requested);
                }
            } else {
                transport_id.into()
            };
            // Metadata uses cached atomic dimensions and must remain available during consent.
            // Wait retains desktop serialization required by M1.
            let _guard = if n == "get_session_info" || name.starts_with("forwarded___") {
                None
            } else {
                Some(s.desktop.lock().await)
            };
            if s.touch(&sid).is_none() {
                if modern {
                    return modern_result(
                        &id,
                        if n == "close_session" {
                            result("Session closed", false)
                        } else {
                            client("Unknown mcpSessionId")
                        },
                        &sid,
                    );
                }
                return StatusCode::NOT_FOUND.into_response();
            }
            if _guard.is_some() {
                // Cleanup workers may still be queued behind this request. Release an
                // expired owner's input under our desktop lock before admitting work.
                let owner = s.held.lock().unwrap().as_ref().map(|h| h.owner.clone());
                if let Some(owner) = owner {
                    let expired = !s.sessions.lock().unwrap().open.contains_key(&owner);
                    if expired {
                        s.release(&owner).await;
                    }
                }
            }
            // Admission and closure are atomic with respect to the session map.
            // The desktop lock prevents another tool close from overtaking admission.
            let active = {
                let mut sessions = s.sessions.lock().unwrap();
                let Some(session) = sessions.open.get_mut(&sid) else {
                    return if modern {
                        modern_result(&id, client("Unknown mcpSessionId"), &sid)
                    } else {
                        StatusCode::NOT_FOUND.into_response()
                    };
                };
                session.active += 1;
                ActiveSession(s.clone(), sid.clone())
            };
            let platform: Arc<dyn Platform> = Arc::new(SessionPlatform {
                inner: s.platform.clone(),
                held: s.held.clone(),
                owner: sid.clone(),
            });
            // PROVISIONAL (not published by AWS): strip the upstream-only session
            // argument; preserve downstream result/error objects.
            let r = if name.starts_with("forwarded___") && s.config.agent_access.forward_mcp_tools {
                let mut forwarded_args = args.clone();
                if modern {
                    forwarded_args
                        .as_object_mut()
                        .unwrap()
                        .remove("mcpSessionId");
                }
                match s.forwarding(&sid) {
                    Some(set) => set.call(name, forwarded_args).await,
                    None => client("Unknown tool"),
                }
            } else if let Some(interface) =
                interface.filter(|_| !taa_tools::dialects::retained(name))
            {
                match interface.translate(
                    name,
                    &args,
                    taa_tools::screen(&s.config, platform.as_ref()),
                ) {
                    Ok((native, translated)) if taa_tools::visible(&native, &s.config, modern) => {
                        taa_tools::execute(&native, &translated, &s.config, platform.clone()).await
                    }
                    Ok(_) => client("Unknown tool"),
                    Err(e) => client(e),
                }
            } else if !taa_tools::NAMES.contains(&n) || !taa_tools::visible(n, &s.config, modern) {
                client("Unknown tool")
            } else if let Err(e) = if n == "execute_sequence" {
                Ok(())
            } else {
                taa_tools::validate_shape(n, &args)
            } {
                client(e)
            } else {
                match n {
                    // PROVISIONAL (not published by AWS): local metadata fields/JSON encoding.
                    "get_session_info" => {
                        let (w, height) = taa_tools::screen(&s.config, s.platform.as_ref());
                        if w == 0 || height == 0 {
                            result("SERVER_ERROR: native screen size unknown; take a screenshot to authorize capture and discover display dimensions",true)
                        } else {
                            result(json!({"mcpSessionId":sid,"hostname":std::env::var("HOSTNAME").or_else(|_|std::env::var("COMPUTERNAME")).unwrap_or_else(|_|"localhost".into()),"os":std::env::consts::OS,"platformBackend":s.platform.name(),"screen":{"width":w,"height":height},"protocolVersion":actual_version,"serverVersion":env!("CARGO_PKG_VERSION")}).to_string(),false)
                        }
                    }
                    // PROVISIONAL (not published by AWS): closed text; expire_streaming_session has no local effect.
                    "close_session" => {
                        s.release(&sid).await;
                        s.close(&sid);
                        result("Session closed", false)
                    }
                    "execute_sequence" => {
                        taa_tools::sequence(&args, &s.config, platform.clone()).await
                    }
                    _ => taa_tools::execute(n, &args, &s.config, platform.clone()).await,
                }
            };
            if !s.config.screenshots.save_dir.is_empty() {
                let dir = s.config.screenshots.save_dir.clone();
                let save_id = sid.clone();
                let saved = r.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    taa_tools::screenshots::save_result(&dir, &save_id, &saved)
                })
                .await;
            }
            drop(active);
            if modern {
                modern_result(&id, r, &sid)
            } else {
                rpc(&id, r)
            }
        }
        _ => request_error(modern, &id, -32601, "Method not found", existing),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn server() -> Arc<Server> {
        let mut c = Config::default();
        c.sessions.idle_timeout_secs = 1;
        Server::new(c, Arc::new(taa_platform::MockPlatform::recording()))
    }
    fn call(
        s: Arc<Server>,
        sid: &str,
        name: &str,
        args: Value,
    ) -> impl std::future::Future<Output = Response> {
        let mut h = HeaderMap::new();
        h.insert("mcp-session-id", sid.parse().unwrap());
        handle_one(
            s,
            h,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":format!("{PREFIX}{name}"),"arguments":args}}),
        )
    }
    fn recording_server() -> (Arc<Server>, Arc<taa_platform::MockPlatform>) {
        let mut config = Config::default();
        config.sessions.max_sessions = 2;
        let mock = Arc::new(taa_platform::MockPlatform::recording());
        (Server::new(config, mock.clone()), mock)
    }
    #[derive(Debug, PartialEq)]
    enum PointerEvent {
        Motion(taa_platform::Point),
        Button(taa_platform::Point, bool),
    }
    struct PartialInput {
        point: taa_platform::Point,
        events: Vec<PointerEvent>,
    }
    impl Default for PartialInput {
        fn default() -> Self {
            Self {
                point: taa_platform::Point { x: 0, y: 0 },
                events: vec![],
            }
        }
    }
    impl taa_platform::input::Input for PartialInput {
        fn motion(&mut self, point: taa_platform::Point) -> Result<(), String> {
            self.point = point;
            self.events.push(PointerEvent::Motion(point));
            Ok(())
        }
        fn button(&mut self, _: taa_platform::Button, down: bool, _: u8) -> Result<(), String> {
            self.events.push(PointerEvent::Button(self.point, down));
            Ok(())
        }
        fn key(&mut self, _: taa_platform::Key, _: bool) -> Result<(), String> {
            Err("X11: unsupported fn key".into())
        }
        fn scroll(&mut self, _: taa_platform::ScrollDirection, _: i64) -> Result<(), String> {
            Ok(())
        }
        fn text(&mut self, _: &str) -> Result<(), String> {
            Ok(())
        }
    }
    #[derive(Default)]
    struct PartialPlatform(Mutex<PartialInput>);
    impl Platform for PartialPlatform {
        fn name(&self) -> &str {
            "partial"
        }
        fn screen_size(&self) -> (u32, u32) {
            (1920, 1080)
        }
        fn capture(&self) -> Result<taa_platform::Frame, String> {
            Err("unused".into())
        }
        fn perform(&self, op: taa_platform::Operation) -> Result<(), String> {
            taa_platform::input::perform(&mut *self.0.lock().unwrap(), op)
        }
    }
    async fn partial_motion_cleanup(expire: bool) {
        use taa_platform::{Button, Key, Operation, Point};
        let platform = Arc::new(PartialPlatform::default());
        let s = Server::new(Config::default(), platform.clone());
        let id = s.open(STATEFUL[2]).unwrap();
        let p = SessionPlatform {
            inner: platform.clone(),
            held: s.held.clone(),
            owner: id.clone(),
        };
        let a = Point { x: 10, y: 20 };
        let b = Point { x: 80, y: 90 };
        p.perform(Operation::Button(a, true, vec![])).unwrap();
        assert!(p
            .perform(Operation::Click(b, Button::Right, 1, vec![Key(0x11000000)]))
            .is_err());
        if expire {
            s.sessions
                .lock()
                .unwrap()
                .open
                .get_mut(&id)
                .unwrap()
                .touched = Instant::now() - Duration::from_secs(1000);
            assert!(s.touch(&id).is_none());
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    if s.held.lock().unwrap().is_none() {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        } else {
            let mut headers = HeaderMap::new();
            headers.insert("host", "localhost".parse().unwrap());
            headers.insert("mcp-session-id", id.parse().unwrap());
            assert_eq!(
                delete(State(s.clone()), headers).await.status(),
                StatusCode::NO_CONTENT
            );
        }
        assert!(s.held.lock().unwrap().is_none());
        assert_eq!(
            platform.0.lock().unwrap().events,
            vec![
                PointerEvent::Motion(a),
                PointerEvent::Button(a, true),
                PointerEvent::Motion(b),
                PointerEvent::Button(b, false),
            ]
        );
    }
    #[tokio::test]
    async fn close_releases_in_place_after_partial_motion_failure() {
        partial_motion_cleanup(false).await;
    }
    #[tokio::test]
    async fn expiry_releases_in_place_after_partial_motion_failure() {
        partial_motion_cleanup(true).await;
    }
    #[tokio::test]
    async fn close_releases_at_current_pointer_after_scroll() {
        let (s, mock) = recording_server();
        let id = s.open(MODERN).unwrap();
        let p = SessionPlatform {
            inner: mock.clone(),
            held: s.held.clone(),
            owner: id.clone(),
        };
        let a = taa_platform::Point { x: 10, y: 20 };
        let b = taa_platform::Point { x: 80, y: 90 };
        p.perform(taa_platform::Operation::Button(a, true, vec![]))
            .unwrap();
        p.perform(taa_platform::Operation::Scroll(
            b,
            taa_platform::ScrollDirection::Down,
            120,
            vec![],
        ))
        .unwrap();
        let _desktop = s.desktop.lock().await;
        s.release(&id).await;
        assert_eq!(
            mock.operations.lock().unwrap().as_slice(),
            &[
                taa_platform::Operation::Button(a, true, vec![]),
                taa_platform::Operation::Scroll(
                    b,
                    taa_platform::ScrollDirection::Down,
                    120,
                    vec![]
                ),
                taa_platform::Operation::ReleaseLeft,
            ]
        );
    }
    #[tokio::test]
    async fn left_click_clears_held_ownership() {
        let (s, mock) = recording_server();
        let id = s.open(MODERN).unwrap();
        let p = SessionPlatform {
            inner: mock.clone(),
            held: s.held.clone(),
            owner: id.clone(),
        };
        let point = taa_platform::Point { x: 10, y: 20 };
        p.perform(taa_platform::Operation::Button(point, true, vec![]))
            .unwrap();
        p.perform(taa_platform::Operation::Click(
            point,
            taa_platform::Button::Left,
            1,
            vec![],
        ))
        .unwrap();
        let _desktop = s.desktop.lock().await;
        s.release(&id).await;
        assert_eq!(mock.operations.lock().unwrap().len(), 2);
    }
    #[tokio::test]
    async fn close_only_releases_current_owner() {
        let (s, mock) = recording_server();
        let a = s.open(MODERN).unwrap();
        let b = s.open(MODERN).unwrap();
        let pa = SessionPlatform {
            inner: mock.clone(),
            held: s.held.clone(),
            owner: a.clone(),
        };
        let pb = SessionPlatform {
            inner: mock.clone(),
            held: s.held.clone(),
            owner: b.clone(),
        };
        let point = taa_platform::Point { x: 10, y: 20 };
        pa.perform(taa_platform::Operation::Button(point, true, vec![]))
            .unwrap();
        pb.perform(taa_platform::Operation::Button(point, false, vec![]))
            .unwrap();
        pb.perform(taa_platform::Operation::Button(point, true, vec![]))
            .unwrap();
        let _desktop = s.desktop.lock().await;
        s.release(&a).await;
        s.close(&a);
        assert_eq!(mock.operations.lock().unwrap().len(), 3);
        s.release(&b).await;
        assert_eq!(mock.operations.lock().unwrap().len(), 4);
    }
    #[tokio::test]
    async fn idle_expiry_releases_owned_button() {
        let (s, mock) = recording_server();
        let id = s.open(MODERN).unwrap();
        let p = SessionPlatform {
            inner: mock.clone(),
            held: s.held.clone(),
            owner: id.clone(),
        };
        p.perform(taa_platform::Operation::Button(
            taa_platform::Point { x: 10, y: 20 },
            true,
            vec![],
        ))
        .unwrap();
        s.sessions
            .lock()
            .unwrap()
            .open
            .get_mut(&id)
            .unwrap()
            .touched = Instant::now() - Duration::from_secs(1000);
        assert!(s.touch(&id).is_none());
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if mock.operations.lock().unwrap().len() == 2 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(s.held.lock().unwrap().is_none());
    }
    #[tokio::test]
    async fn modern_new_releases_expired_button_before_move() {
        let mut config = Config::default();
        config.agent_access.screen_resolution = "native".into();
        let mock = Arc::new(taa_platform::MockPlatform::recording());
        let s = Server::new(config, mock.clone());
        let id = s.open(MODERN).unwrap();
        let point = taa_platform::Point { x: 10, y: 20 };
        SessionPlatform {
            inner: mock.clone(),
            held: s.held.clone(),
            owner: id.clone(),
        }
        .perform(taa_platform::Operation::Button(point, true, vec![]))
        .unwrap();
        s.sessions
            .lock()
            .unwrap()
            .open
            .get_mut(&id)
            .unwrap()
            .touched = Instant::now() - Duration::from_secs(1000);
        let mut headers = HeaderMap::new();
        headers.insert("mcp-protocol-version", MODERN.parse().unwrap());
        headers.insert("mcp-method", "tools/call".parse().unwrap());
        headers.insert("mcp-name", format!("{PREFIX}move_pointer").parse().unwrap());
        // Current-thread runtime: the spawned cleanup cannot acquire the lock
        // before this request does. No sleeps or scheduler timing assumptions.
        let response = handle_one(
            s,
            headers,
            json!({
                "jsonrpc":"2.0", "id":1, "method":"tools/call",
                "params": {"name":format!("{PREFIX}move_pointer"),
                    "arguments":{"mcpSessionId":"NEW","x":30,"y":40},
                    "_meta":{"io.modelcontextprotocol/protocolVersion":MODERN,
                        "io.modelcontextprotocol/clientCapabilities":{}}}
            }),
        )
        .await;
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let result: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(result["result"]["isError"], false, "{result}");
        assert_eq!(
            mock.operations.lock().unwrap().as_slice(),
            &[
                taa_platform::Operation::Button(point, true, vec![]),
                taa_platform::Operation::ReleaseLeft,
                taa_platform::Operation::Move(taa_platform::Point { x: 30, y: 40 }),
            ]
        );
    }
    #[tokio::test]
    async fn sequence_press_is_released_by_delete() {
        let (s, mock) = recording_server();
        let id = s.open(STATEFUL[2]).unwrap();
        let response = call(
            s.clone(),
            &id,
            "execute_sequence",
            json!({"actions":[
                {"type":"left_mouse_down","x":10,"y":20},
                {"type":"wait","duration":0.001}
            ]}),
        )
        .await;
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["result"]["isError"], false);
        let mut headers = HeaderMap::new();
        headers.insert("host", "localhost".parse().unwrap());
        headers.insert("mcp-session-id", id.parse().unwrap());
        assert_eq!(
            delete(State(s.clone()), headers).await.status(),
            StatusCode::NO_CONTENT
        );
        let ops = mock.operations.lock().unwrap();
        assert_eq!(ops.len(), 2);
        assert!(matches!(
            (&ops[0], &ops[1]),
            (
                taa_platform::Operation::Button(_, true, _),
                taa_platform::Operation::ReleaseLeft
            )
        ));
    }
    #[tokio::test]
    async fn metadata_does_not_wait_for_desktop_consent() {
        let s = server();
        let sid = s.open(STATEFUL[2]).unwrap();
        let _pending_consent = s.desktop.lock().await;
        let response = tokio::time::timeout(
            Duration::from_millis(100),
            call(s.clone(), &sid, "get_session_info", json!({})),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(s.sessions.lock().unwrap().open[&sid].active, 0);
    }
    #[tokio::test]
    async fn native_metadata_returns_guidance_during_pending_consent() {
        struct Pending;
        impl taa_platform::Platform for Pending {
            fn name(&self) -> &str {
                "wayland-portal"
            }
            fn screen_size(&self) -> (u32, u32) {
                (0, 0)
            }
            fn capture(&self) -> Result<taa_platform::Frame, String> {
                panic!("metadata must never capture")
            }
            fn perform(&self, _: taa_platform::Operation) -> Result<(), String> {
                panic!("metadata must never inject")
            }
        }
        let mut config = Config::default();
        config.agent_access.screen_resolution = "native".into();
        let s = Server::new(config, Arc::new(Pending));
        let sid = s.open(STATEFUL[2]).unwrap();
        let _consent = s.desktop.lock().await;
        let response = tokio::time::timeout(
            Duration::from_millis(100),
            call(s.clone(), &sid, "get_session_info", json!({})),
        )
        .await
        .unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["result"]["isError"], true);
        assert!(value["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("take a screenshot"));
    }
    #[tokio::test]
    async fn queued_work_cannot_run_after_close() {
        let s = server();
        let sid = s.open(STATEFUL[2]).unwrap();
        let desktop = s.desktop.lock().await;
        let task = tokio::spawn(call(s.clone(), &sid, "wait", json!({"duration":0.001})));
        tokio::task::yield_now().await;
        assert!(!task.is_finished());
        s.close(&sid); // DELETE may close while tool work is waiting for the desktop.
        drop(desktop);
        assert_eq!(task.await.unwrap().status(), StatusCode::NOT_FOUND);
    }
    #[tokio::test]
    async fn active_work_does_not_expire_and_serializes_desktop() {
        let s = server();
        let sid = s.open(STATEFUL[2]).unwrap();
        let task = tokio::spawn(call(s.clone(), &sid, "wait", json!({"duration":1.2})));
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if s.sessions.lock().unwrap().open[&sid].active == 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(s.desktop.try_lock().is_err());
        tokio::time::sleep(Duration::from_millis(1050)).await;
        assert!(s.touch(&sid).is_some());
        assert!(s.open(STATEFUL[2]).is_err());
        assert_eq!(task.await.unwrap().status(), StatusCode::OK);
        assert_eq!(s.sessions.lock().unwrap().open[&sid].active, 0);
        assert!(s.touch(&sid).is_some());
    }
    #[test]
    fn tombstones_expire_after_retry_window() {
        let s = server();
        let sid = s.open(MODERN).unwrap();
        s.close(&sid);
        assert!(s.sessions.lock().unwrap().closed.contains_key(&sid));
        s.sessions
            .lock()
            .unwrap()
            .closed
            .insert(sid.clone(), Instant::now() - Duration::from_secs(301));
        assert!(s.touch(&sid).is_none());
        assert!(!s.sessions.lock().unwrap().closed.contains_key(&sid));
    }
}
