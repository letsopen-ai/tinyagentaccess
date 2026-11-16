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
    io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader},
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
    fn spawn(spec: &ServerSpec) -> Result<Arc<Self>, Failure> {
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
                        let _ = write(&stdin, &alive, json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not found"}})).await;
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
        Ok(client)
    }
    async fn initialize(self: &Arc<Self>, limits: Timeouts) -> Result<(), Failure> {
        let client = self;
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
            &client.alive,
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        )
        .await?;
        Ok(())
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
        let mut written = false;
        let outcome = tokio::time::timeout(budget, async {
            write(
                &self.stdin,
                &self.alive,
                json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
            )
            .await?;
            written = true;
            rx.await.map_err(|_| Failure::Unavailable)?
        })
        .await;
        match outcome {
            Ok(v) => v,
            Err(_) => {
                // §8: cancel a timed-out downstream request. Bound even a blocked pipe write.
                if written {
                    let _ = tokio::time::timeout(Duration::from_millis(50), write(&self.stdin, &self.alive, json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id,"reason":"timeout"}}))).await;
                }
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
// An interrupted write poisons and closes the pipe before releasing the mutex.
// No later frame (including cancellation) may follow a partial JSON line.
async fn write<W: AsyncWrite + Unpin>(
    stdin: &tokio::sync::Mutex<Option<W>>,
    alive: &AtomicBool,
    value: Value,
) -> Result<(), Failure> {
    let mut stdin = stdin.lock().await;
    if stdin.is_none() {
        return Err(Failure::Unavailable);
    }
    struct FrameGuard<'a, W> {
        pipe: &'a mut Option<W>,
        alive: &'a AtomicBool,
        complete: bool,
    }
    impl<W> Drop for FrameGuard<'_, W> {
        fn drop(&mut self) {
            if !self.complete {
                self.alive.store(false, Ordering::SeqCst);
                self.pipe.take();
            }
        }
    }
    let mut frame = FrameGuard {
        pipe: &mut stdin,
        alive,
        complete: false,
    };
    let mut bytes = serde_json::to_vec(&value).unwrap();
    bytes.push(b'\n');
    frame
        .pipe
        .as_mut()
        .unwrap()
        .write_all(&bytes)
        .await
        .map_err(|_| Failure::Unavailable)?;
    frame.complete = true;
    Ok(())
}
#[derive(Clone)]
struct Entry {
    spec: ServerSpec,
    client: Option<Arc<Client>>,
    manifest: Vec<Value>,
}
struct State {
    discovering: Vec<Arc<Client>>,
    entries: Option<Vec<Entry>>,
    routes: HashMap<String, (usize, String)>,
}
/// PROVISIONAL (not published by AWS): per-session lazy config/init, manifest refresh,
/// exact cached routes, crash respawn on list, local failures and schema/result relay policy.
pub struct ForwardingSet {
    path: std::path::PathBuf,
    limits: Timeouts,
    state: tokio::sync::Mutex<State>,
    refresh_lock: tokio::sync::Mutex<()>,
    refresh_generation: AtomicU64,
    closed: AtomicBool,
    stop: tokio::sync::Notify,
}
impl ForwardingSet {
    pub fn new(path: impl AsRef<Path>, limits: Timeouts) -> Arc<Self> {
        Arc::new(Self {
            path: path.as_ref().into(),
            limits,
            state: tokio::sync::Mutex::new(State {
                discovering: vec![],
                entries: None,
                routes: HashMap::new(),
            }),
            refresh_lock: tokio::sync::Mutex::new(()),
            refresh_generation: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            stop: tokio::sync::Notify::new(),
        })
    }
    async fn refresh(&self, observed: Option<u64>) -> Vec<Value> {
        let stopped = self.stop.notified();
        tokio::pin!(stopped);
        stopped.as_mut().enable();
        if self.closed.load(Ordering::SeqCst) {
            return vec![];
        }
        tokio::select! {
            biased;
            _ = stopped => vec![],
            tools = self.refresh_inner(observed) => tools,
        }
    }
    async fn refresh_inner(&self, observed: Option<u64>) -> Vec<Value> {
        let _refresh = self.refresh_lock.lock().await;
        // A concurrent completed refresh already retried this route miss.
        if observed
            .is_some_and(|generation| generation != self.refresh_generation.load(Ordering::SeqCst))
        {
            return vec![];
        }
        let (mut entries, abandoned) = {
            let mut state = self.state.lock().await;
            (
                state.entries.clone(),
                std::mem::take(&mut state.discovering),
            )
        };
        for client in abandoned {
            client.close(self.limits.graceful).await;
        }
        if self.closed.load(Ordering::SeqCst) {
            return vec![];
        }
        if entries.is_none() {
            let parsed = match tokio::fs::read(&self.path).await {
                Ok(bytes) => parse_config(&bytes),
                Err(e) => Err(e.to_string()),
            };
            // PROVISIONAL (not published by AWS): one config warning per session,
            // leaving native tools available and caching the failed load.
            entries = Some(match parsed {
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
        for (index, entry) in entries.as_mut().unwrap().iter_mut().enumerate() {
            if entry
                .client
                .as_ref()
                .is_none_or(|c| !c.alive.load(Ordering::SeqCst))
            {
                if let Some(old) = entry.client.take() {
                    old.close(self.limits.graceful).await;
                }
                let spawned = match Client::spawn(&entry.spec) {
                    Ok(client) => {
                        let mut state = self.state.lock().await;
                        if self.closed.load(Ordering::SeqCst) {
                            drop(state);
                            client.close(self.limits.graceful).await;
                            return vec![];
                        }
                        state.discovering.push(client.clone());
                        drop(state);
                        let result = client.initialize(self.limits).await;
                        if result.is_err() {
                            client.close(self.limits.graceful).await;
                        }
                        if result.is_err() {
                            self.state
                                .lock()
                                .await
                                .discovering
                                .retain(|c| !Arc::ptr_eq(c, &client));
                        }
                        result.map(|()| client)
                    }
                    Err(e) => Err(e),
                };
                entry.client = match spawned {
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
        for (index, entry) in entries.as_ref().unwrap().iter().enumerate() {
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
        let mut state = self.state.lock().await;
        if self.closed.load(Ordering::SeqCst) {
            drop(state);
            for entry in entries.unwrap_or_default() {
                if let Some(client) = entry.client {
                    client.close(self.limits.graceful).await;
                }
            }
            return vec![];
        }
        state.discovering.clear();
        state.entries = entries;
        state.routes = routes;
        self.refresh_generation.fetch_add(1, Ordering::SeqCst);
        tools
    }
    pub async fn list(&self) -> Vec<Value> {
        self.refresh(None).await
    }
    pub async fn call(&self, name: &str, args: Value) -> Value {
        if !args.is_object() {
            return failure_result("CLIENT_ERROR: arguments must be an object");
        }
        let observed = self.refresh_generation.load(Ordering::SeqCst);
        if !self.state.lock().await.routes.contains_key(name) {
            self.refresh(Some(observed)).await;
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
            let mut clients = std::mem::take(&mut state.discovering);
            clients.extend(
                state
                    .entries
                    .take()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|e| e.client),
            );
            for client in clients {
                let grace = self.limits.graceful;
                tasks.spawn(async move { client.close(grace).await });
            }
            state.routes.clear();
            drop(state);
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn route_misses_share_completed_refresh_but_lists_always_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"mcpServers":{}}"#).unwrap();
        let set = ForwardingSet::new(path, Timeouts::default());
        let observed = set.refresh_generation.load(Ordering::SeqCst);
        set.refresh(Some(observed)).await;
        assert_eq!(set.refresh_generation.load(Ordering::SeqCst), observed + 1);
        // Model a second miss that observed the same generation before waiting.
        set.refresh(Some(observed)).await;
        assert_eq!(set.refresh_generation.load(Ordering::SeqCst), observed + 1);
        set.list().await;
        assert_eq!(set.refresh_generation.load(Ordering::SeqCst), observed + 2);
        set.teardown().await;
    }

    #[tokio::test]
    async fn interrupted_frame_closes_pipe_before_next_write() {
        let (pipe, mut reader) = tokio::io::duplex(64);
        let stdin = tokio::sync::Mutex::new(Some(pipe));
        let alive = AtomicBool::new(true);
        assert!(tokio::time::timeout(
            Duration::from_millis(20),
            write(&stdin, &alive, json!({"payload":"x".repeat(4096)}))
        )
        .await
        .is_err());
        assert!(!alive.load(Ordering::SeqCst));
        assert!(stdin.lock().await.is_none());
        assert!(
            write(&stdin, &alive, json!({"method":"notifications/cancelled"}))
                .await
                .is_err()
        );
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(bytes.len(), 64);
        assert!(!bytes.contains(&b'\n'));
    }
}
