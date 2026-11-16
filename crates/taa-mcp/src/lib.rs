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
#[derive(Debug, Clone, Copy)]
pub enum Interface {
    Anthropic20250124,
    Anthropic20251124,
    AnthropicToolset20260801,
    OpenaiV1,
    Google3x,
}
impl Interface {
    pub fn parse(s: &str) -> Result<Option<Self>, String> {
        Ok(match s {
            "" => None,
            "ANTHROPIC_COMPUTER_20250124" => Some(Self::Anthropic20250124),
            "ANTHROPIC_COMPUTER_20251124" => Some(Self::Anthropic20251124),
            "ANTHROPIC_COMPUTER_TOOLSET_20260801" => Some(Self::AnthropicToolset20260801),
            "OPENAI_COMPUTER_V1" => Some(Self::OpenaiV1),
            "GOOGLE_COMPUTER_USE_3X_DESKTOP_V1" => Some(Self::Google3x),
            _ => return Err("Unknown computer-use interface id".into()),
        })
    }
}
struct Session {
    version: String,
    touched: Instant,
    active: usize,
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
    desktop: tokio::sync::Mutex<()>,
}
impl Server {
    pub fn new(config: Config, platform: Arc<dyn Platform>) -> Arc<Self> {
        Arc::new(Self {
            config,
            platform,
            sessions: Mutex::new(Sessions::default()),
            desktop: tokio::sync::Mutex::new(()),
        })
    }
    fn cleanup(&self, s: &mut Sessions) {
        s.open.retain(|_, v| {
            v.active > 0
                || v.touched.elapsed() < Duration::from_secs(self.config.sessions.idle_timeout_secs)
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
    fn close(&self, id: &str) {
        let mut s = self.sessions.lock().unwrap();
        s.open.remove(id);
        s.closed.insert(id.into(), Instant::now());
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
        return (StatusCode::BAD_REQUEST, Json(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32022,"message":"Unsupported protocol version","data":{"supported":[MODERN],"requested":version}}}))).into_response();
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
    if matches!(method, "tools/list" | "tools/call") {
        match Interface::parse(header(&h, "x-amzn-agentaccess-computer-use-interface-id")) {
            Err(e) => return request_error(modern, &id, -32602, &e, existing),
            Ok(Some(_)) => {
                return request_error(
                    modern,
                    &id,
                    -32603,
                    "Computer-use interface is not implemented yet",
                    existing,
                )
            }
            _ => (),
        }
    }
    match method {
        "tools/list" => {
            let mut r = taa_tools::list(&s.config, modern);
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
            let _guard = s.desktop.lock().await;
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
            let r = if !taa_tools::NAMES.contains(&n) || !taa_tools::visible(n, &s.config, modern) {
                client("Unknown tool")
            } else if let Err(e) =
                taa_tools::validate(n, &args, taa_tools::screen(&s.config, s.platform.as_ref()))
            {
                client(e)
            } else {
                match n {
                    "get_session_info" => {
                        let (w, height) = taa_tools::screen(&s.config, s.platform.as_ref());
                        result(json!({"mcpSessionId":sid,"hostname":std::env::var("HOSTNAME").or_else(|_|std::env::var("COMPUTERNAME")).unwrap_or_else(|_|"localhost".into()),"os":std::env::consts::OS,"platformBackend":s.platform.name(),"screen":{"width":w,"height":height},"protocolVersion":actual_version,"serverVersion":env!("CARGO_PKG_VERSION")}).to_string(),false)
                    }
                    "close_session" => {
                        s.close(&sid);
                        result("Session closed", false)
                    }
                    "execute_sequence" => {
                        taa_tools::sequence(&args, &s.config, s.platform.as_ref()).await
                    }
                    _ => taa_tools::execute(n, &args, &s.config, s.platform.as_ref()).await,
                }
            };
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
