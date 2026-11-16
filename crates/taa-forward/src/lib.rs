//! Stdio forwarding. AWS facts refer to docs/aws-parity.md §8.
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::Path,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::oneshot,
};

#[derive(Clone, Debug)]
pub struct ServerSpec {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
}
#[derive(Debug)]
pub struct ManifestConfig {
    pub servers: Vec<ServerSpec>,
    pub warnings: Vec<String>,
}
/// §8: reject the whole file on encoding/shape errors; ignore unknown fields.
pub fn parse_config(bytes: &[u8]) -> Result<ManifestConfig, String> {
    for bom in [
        &b"\xff\xfe\0\0"[..],
        &b"\0\0\xfe\xff"[..],
        &b"\xef\xbb\xbf"[..],
        &b"\xff\xfe"[..],
        &b"\xfe\xff"[..],
    ] {
        if bytes.starts_with(bom) {
            return Err("forwarding config must be UTF-8 without BOM".into());
        }
    }
    let raw = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let value: Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    let root = value.as_object().ok_or("config root must be an object")?;
    root.get("mcpServers")
        .and_then(Value::as_object)
        .ok_or("mcpServers must be an object")?;
    let mut config = ManifestConfig {
        servers: vec![],
        warnings: vec![],
    };
    for key in root.keys().filter(|k| *k != "mcpServers") {
        config.warnings.push(format!("ignored config key {key}"));
    }
    // Preserve configuration order without changing serde_json's map representation globally.
    #[derive(serde::Deserialize)]
    struct OrderedRoot {
        #[serde(rename = "mcpServers")]
        servers: Ordered,
    }
    struct Ordered(Vec<(String, Value)>);
    impl<'de> serde::Deserialize<'de> for Ordered {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct Visitor;
            impl<'de> serde::de::Visitor<'de> for Visitor {
                type Value = Ordered;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str("server map")
                }
                fn visit_map<M: serde::de::MapAccess<'de>>(
                    self,
                    mut m: M,
                ) -> Result<Ordered, M::Error> {
                    let mut entries = vec![];
                    while let Some(pair) = m.next_entry()? {
                        entries.push(pair);
                    }
                    Ok(Ordered(entries))
                }
            }
            d.deserialize_map(Visitor)
        }
    }
    let ordered: OrderedRoot = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    for (name, entry) in &ordered.servers.0 {
        let entry = entry.as_object().ok_or("server entry must be an object")?;
        let command = entry
            .get("command")
            .and_then(Value::as_str)
            .ok_or("command must be a string")?;
        let args = match entry.get("args") {
            None => vec![],
            Some(v) => v
                .as_array()
                .ok_or("args must be an array")?
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .ok_or("args must contain strings")
                })
                .collect::<Result<Vec<_>, _>>()?,
        };
        for key in entry
            .keys()
            .filter(|k| !["command", "args"].contains(&k.as_str()))
        {
            config
                .warnings
                .push(format!("server '{name}': ignored key {key}"));
        }
        if name.contains("___") {
            config
                .warnings
                .push(format!("ambiguous server name '{name}' contains ___"));
        }
        config.servers.push(ServerSpec {
            name: name.clone(),
            command: command.into(),
            args,
        });
    }
    Ok(config)
}
pub fn forwarded_name(server: &str, tool: &str) -> String {
    format!("forwarded___{server}___{tool}")
}
#[derive(Clone, Copy)]
pub struct Timeouts {
    pub request: Duration,
    pub teardown: Duration,
    pub graceful: Duration,
    pub initialize: Duration,
}
impl Default for Timeouts {
    fn default() -> Self {
        Self {
            request: Duration::from_secs(5),
            teardown: Duration::from_secs(15),
            graceful: Duration::from_secs(1),
            // PROVISIONAL (not published by AWS): safety cap; §8 leaves initialize unbounded.
            initialize: Duration::from_secs(60),
        }
    }
}
#[derive(Debug)]
enum Failure {
    Unavailable,
    Timeout,
    Rpc(i64, String),
}
type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, Failure>>>>>;
struct Client {
    stdin: Arc<tokio::sync::Mutex<Option<ChildStdin>>>,
    child: tokio::sync::Mutex<Child>,
    pending: Pending,
    alive: Arc<AtomicBool>,
    next: AtomicU64,
}
impl Client {
    async fn spawn(spec: &ServerSpec, limits: Timeouts) -> Result<Arc<Self>, Failure> {
        // §8: command + args, inherited environment, stdio only; kill_on_drop backstop.
        let mut child = Command::new(&spec.command)
            .args(&spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| Failure::Unavailable)?;
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let stdin = Arc::new(tokio::sync::Mutex::new(child.stdin.take()));
        let pending: Pending = Arc::default();
        let alive = Arc::new(AtomicBool::new(true));
        let client = Arc::new(Self {
            stdin: stdin.clone(),
            child: tokio::sync::Mutex::new(child),
            pending: pending.clone(),
            alive: alive.clone(),
            next: AtomicU64::new(1),
        });
        // PROVISIONAL (not published by AWS): line-prefixed stderr, warning-only
        // non-JSON stdout, and no server-to-client capabilities.
        let name = spec.name.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                eprintln!("[forwarded:{name}] {line}");
            }
        });
        let name = spec.name.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let message: Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(_) => {
                        eprintln!("warning: [forwarded:{name}] ignored non-JSON stdout");
                        continue;
                    }
                };
                // PROVISIONAL (not published by AWS): unsupported server requests get -32601; notifications ignored.
                if message.get("method").is_some() {
                    if let Some(id) = message.get("id") {
                        let _ = write(&stdin, json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not found"}})).await;
                    }
                } else if let Some(id) = message["id"].as_u64() {
                    if let Some(tx) = pending.lock().unwrap().remove(&id) {
                        let outcome = if let Some(e) = message.get("error") {
                            Err(Failure::Rpc(
                                e["code"].as_i64().unwrap_or(-32603),
                                e["message"].as_str().unwrap_or("Downstream error").into(),
                            ))
                        } else {
                            Ok(message["result"].clone())
                        };
                        let _ = tx.send(outcome);
                    }
                }
            }
            alive.store(false, Ordering::SeqCst);
            pending.lock().unwrap().clear();
        });
        // PROVISIONAL (not published by AWS): offer 2025-06-18, accept known versions.
        let initialized = client.request("initialize", json!({"protocolVersion":"2025-06-18","clientInfo":{"name":"tinyAgentAccess","version":env!("CARGO_PKG_VERSION")},"capabilities":{}}), limits.initialize).await?;
        if ![
            "2024-11-05",
            "2025-03-26",
            "2025-06-18",
            "2025-11-25",
            "2026-07-28",
        ]
        .contains(&initialized["protocolVersion"].as_str().unwrap_or(""))
        {
            client.close(limits.graceful).await;
            return Err(Failure::Unavailable);
        }
        write(
            &client.stdin,
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        )
        .await?;
        Ok(client)
    }
    async fn request(
        &self,
        method: &str,
        params: Value,
        budget: Duration,
    ) -> Result<Value, Failure> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(Failure::Unavailable);
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        struct RemovePending(Pending, u64);
        impl Drop for RemovePending {
            fn drop(&mut self) {
                self.0.lock().unwrap().remove(&self.1);
            }
        }
        let _remove = RemovePending(self.pending.clone(), id);
        let outcome = tokio::time::timeout(budget, async {
            write(
                &self.stdin,
                json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
            )
            .await?;
            rx.await.map_err(|_| Failure::Unavailable)?
        })
        .await;
        self.pending.lock().unwrap().remove(&id);
        match outcome {
            Ok(v) => v,
            Err(_) => {
                // §8: cancel a timed-out downstream request. Bound even a blocked pipe write.
                let _ = tokio::time::timeout(Duration::from_millis(50), write(&self.stdin, json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id,"reason":"timeout"}}))).await;
                Err(Failure::Timeout)
            }
        }
    }
    async fn close(&self, grace: Duration) {
        self.alive.store(false, Ordering::SeqCst);
        self.pending.lock().unwrap().clear();
        // §8: the grace budget covers closing stdin as well as waiting. A blocked
        // writer must not hold teardown hostage; killing the child releases its pipe.
        let graceful = async {
            self.stdin.lock().await.take();
            self.child.lock().await.wait().await
        };
        if !matches!(tokio::time::timeout(grace, graceful).await, Ok(Ok(_))) {
            let mut child = self.child.lock().await;
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
    }
}
async fn write(
    stdin: &tokio::sync::Mutex<Option<ChildStdin>>,
    value: Value,
) -> Result<(), Failure> {
    let mut stdin = stdin.lock().await;
    let pipe = stdin.as_mut().ok_or(Failure::Unavailable)?;
    let mut bytes = serde_json::to_vec(&value).unwrap();
    bytes.push(b'\n');
    pipe.write_all(&bytes)
        .await
        .map_err(|_| Failure::Unavailable)
}
struct Entry {
    spec: ServerSpec,
    client: Option<Arc<Client>>,
    manifest: Vec<Value>,
}
struct State {
    entries: Option<Vec<Entry>>,
    routes: HashMap<String, (usize, String)>,
}
/// PROVISIONAL (not published by AWS): per-session lazy config/init, manifest refresh,
/// exact cached routes, crash respawn on list, local failures and schema/result relay policy.
pub struct ForwardingSet {
    path: std::path::PathBuf,
    limits: Timeouts,
    state: tokio::sync::Mutex<State>,
    closed: AtomicBool,
    stop: tokio::sync::Notify,
}
impl ForwardingSet {
    pub fn new(path: impl AsRef<Path>, limits: Timeouts) -> Arc<Self> {
        Arc::new(Self {
            path: path.as_ref().into(),
            limits,
            state: tokio::sync::Mutex::new(State {
                entries: None,
                routes: HashMap::new(),
            }),
            closed: AtomicBool::new(false),
            stop: tokio::sync::Notify::new(),
        })
    }
    async fn refresh(&self) -> Vec<Value> {
        let stopped = self.stop.notified();
        tokio::pin!(stopped);
        stopped.as_mut().enable();
        if self.closed.load(Ordering::SeqCst) {
            return vec![];
        }
        tokio::select! {
            biased;
            _ = stopped => vec![],
            tools = self.refresh_inner() => tools,
        }
    }
    async fn refresh_inner(&self) -> Vec<Value> {
        let mut state = self.state.lock().await;
        if self.closed.load(Ordering::SeqCst) {
            return vec![];
        }
        if state.entries.is_none() {
            let parsed = match tokio::fs::read(&self.path).await {
                Ok(bytes) => parse_config(&bytes),
                Err(e) => Err(e.to_string()),
            };
            // PROVISIONAL (not published by AWS): one config warning per session,
            // leaving native tools available and caching the failed load.
            state.entries = Some(match parsed {
                Ok(config) => {
                    for warning in config.warnings {
                        eprintln!("warning: {warning}");
                    }
                    config
                        .servers
                        .into_iter()
                        .map(|spec| Entry {
                            spec,
                            client: None,
                            manifest: vec![],
                        })
                        .collect()
                }
                Err(e) => {
                    eprintln!("warning: forwarding disabled for session: {e}");
                    vec![]
                }
            });
        }
        let mut successful = std::collections::HashSet::new();
        let mut routes = HashMap::new();
        let mut tools = vec![];
        for (index, entry) in state.entries.as_mut().unwrap().iter_mut().enumerate() {
            if entry
                .client
                .as_ref()
                .is_none_or(|c| !c.alive.load(Ordering::SeqCst))
            {
                if let Some(old) = entry.client.take() {
                    old.close(self.limits.graceful).await;
                }
                entry.client = match Client::spawn(&entry.spec, self.limits).await {
                    Ok(c) => Some(c),
                    Err(_) => {
                        eprintln!(
                            "warning: forwarded server '{}' failed to initialize",
                            entry.spec.name
                        );
                        None
                    }
                };
            }
            let Some(client) = &entry.client else {
                continue;
            };
            // §8: listing has a 5s budget, excluding spawn/init.
            // PROVISIONAL (not published by AWS): follow all nextCursor pages
            // within that same budget; reject cycles.
            let deadline = tokio::time::Instant::now() + self.limits.request;
            let mut collected = vec![];
            let mut cursor = None;
            let mut seen = std::collections::HashSet::new();
            let listed = loop {
                let params = cursor.as_ref().map_or(json!({}), |c| json!({"cursor":c}));
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                match client.request("tools/list", params, remaining).await {
                    Ok(page) => {
                        let Some(page_tools) = page["tools"].as_array() else {
                            break false;
                        };
                        collected.extend(page_tools.iter().cloned());
                        cursor = page["nextCursor"].as_str().map(str::to_owned);
                        match &cursor {
                            None => break true,
                            Some(c) if !seen.insert(c.clone()) => break false,
                            _ => (),
                        }
                    }
                    Err(_) => break false,
                }
            };
            if !listed {
                continue;
            }
            entry.manifest = collected;
            successful.insert(index);
        }
        // PROVISIONAL (not published by AWS): retain each latest successful manifest
        // for routing even when a later listing fails. Failed listings expose no tools.
        for (index, entry) in state.entries.as_ref().unwrap().iter().enumerate() {
            for mut tool in entry.manifest.clone() {
                let Some(original) = tool["name"].as_str().map(str::to_owned) else {
                    continue;
                };
                if original.contains("___") {
                    eprintln!("warning: ambiguous forwarded tool '{original}' contains ___");
                }
                let name = forwarded_name(&entry.spec.name, &original);
                // PROVISIONAL (not published by AWS): first config entry wins collisions.
                if routes.contains_key(&name) {
                    eprintln!("warning: duplicate forwarded name '{name}'; first wins");
                    continue;
                }
                tool["name"] = name.clone().into();
                routes.insert(name, (index, original));
                if successful.contains(&index) {
                    tools.push(tool);
                }
            }
        }
        state.routes = routes;
        tools
    }
    pub async fn list(&self) -> Vec<Value> {
        self.refresh().await
    }
    pub async fn call(&self, name: &str, args: Value) -> Value {
        if !args.is_object() {
            return failure_result("CLIENT_ERROR: arguments must be an object");
        }
        if !self.state.lock().await.routes.contains_key(name) {
            self.refresh().await;
        }
        let route = {
            let state = self.state.lock().await;
            state.routes.get(name).map(|(index, original)| {
                let entry = &state.entries.as_ref().unwrap()[*index];
                (
                    entry.client.clone(),
                    entry.spec.name.clone(),
                    original.clone(),
                )
            })
        };
        let Some((client, server, original)) = route else {
            return failure_result("CLIENT_ERROR: Unknown tool");
        };
        let Some(client) = client else {
            return failure_result(&format!(
                "SERVER_ERROR: forwarded server '{server}' is unavailable"
            ));
        };
        match client
            .request(
                "tools/call",
                json!({"name":original,"arguments":args}),
                self.limits.request,
            )
            .await
        {
            Ok(v) => v,
            Err(Failure::Rpc(code, message)) => failure_result(&rpc_error(code, &message)),
            Err(Failure::Timeout) => {
                failure_result("SERVER_ERROR: forwarded tool call timed out after 5s")
            }
            Err(Failure::Unavailable) => failure_result(&format!(
                "SERVER_ERROR: forwarded server '{server}' is unavailable"
            )),
        }
    }
    pub async fn teardown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.stop.notify_waiters();
        let work = async {
            let mut state = self.state.lock().await;
            let mut tasks = tokio::task::JoinSet::new();
            for entry in state.entries.take().unwrap_or_default() {
                if let Some(client) = entry.client {
                    let grace = self.limits.graceful;
                    tasks.spawn(async move { client.close(grace).await });
                }
            }
            state.routes.clear();
            while tasks.join_next().await.is_some() {}
        };
        // §8: concurrent teardown within 15s, per-server 1s graceful close.
        let _ = tokio::time::timeout(self.limits.teardown, work).await;
    }
}
// PROVISIONAL (not published by AWS): locally generated tool-error mapping.
pub fn rpc_error(code: i64, message: &str) -> String {
    format!(
        "{}: {message}",
        if matches!(code, -32602..=-32600) {
            "CLIENT_ERROR"
        } else {
            "SERVER_ERROR"
        }
    )
}
fn failure_result(message: &str) -> Value {
    json!({"content":[{"type":"text","text":message}],"isError":true})
}
