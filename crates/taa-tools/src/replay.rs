//! Local evidence only. Timing, layout and record shapes are PROVISIONAL (D-evidence).
use base64::Engine;
use serde_json::{json, Value};
use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct Recorder {
    root: String,
    session: String,
    state: Mutex<ReplayState>,
}
#[derive(Default)]
struct ReplayState {
    disk: Option<(PathBuf, File)>,
    seq: u64,
    pending: usize,
    ended: bool,
}
#[derive(Clone)]
pub struct Context {
    pub recorder: Arc<Recorder>,
    pub tool: String,
    pub arguments: Value,
}
pub struct Event {
    recorder: Arc<Recorder>,
    metadata: Value,
    finished: bool,
}
fn private_file(path: &std::path::Path) -> std::io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
}
fn warn<T>(r: Result<T, Box<dyn std::error::Error>>) -> Option<T> {
    match r {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!("warning: replay save failed: {e}");
            None
        }
    }
}
// Best-effort named-secret omission; arbitrary desktop pixels/text are not redacted.
fn sanitize(v: &mut Value) {
    match v {
        Value::Object(o) => {
            if o.get("type").is_some_and(|v| v == "image") {
                o.remove("data");
            }
            for (key, value) in o {
                let key: String = key
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect();
                if [
                    "password",
                    "passwd",
                    "secret",
                    "token",
                    "accesstoken",
                    "refreshtoken",
                    "apikey",
                    "authorization",
                    "credential",
                    "credentials",
                ]
                .contains(&key.as_str())
                {
                    *value = json!("[omitted sensitive field]");
                } else {
                    sanitize(value);
                }
            }
        }
        Value::Array(a) => {
            for v in a {
                sanitize(v);
            }
        }
        _ => (),
    }
}
// Cap opaque downstream payloads and omit image base64 from responses.
fn bounded(v: &Value) -> Value {
    let mut v = v.clone();
    sanitize(&mut v);
    let s = v.to_string();
    if s.len() <= 65536 {
        v
    } else {
        json!({"truncated":true,"original_bytes":s.len(),"prefix":s.chars().take(16384).collect::<String>()})
    }
}
impl Recorder {
    pub fn new(root: String, session: String) -> Arc<Self> {
        Arc::new(Self {
            root,
            session,
            state: Mutex::new(ReplayState::default()),
        })
    }
    pub fn begin(
        self: &Arc<Self>,
        tool: &str,
        arguments: &Value,
        native: Option<&str>,
        frame: Option<&Value>,
    ) -> Option<Event> {
        warn((|| {
            let mut s = self.state.lock().unwrap();
            if s.ended {
                return Err("replay session ended".into());
            }
            if self.session.is_empty()
                || !self
                    .session
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                return Err("invalid replay session component".into());
            }
            let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
            if s.disk.is_none() {
                let (y, m, d) = crate::screenshots::date((millis / 86_400_000) as i64);
                let root: PathBuf = std::path::Path::new(&self.root).components().collect();
                let path = root
                    .join(format!(
                        "agentaccess/replay/year={y:04}/month={m:02}/day={d:02}"
                    ))
                    .join(&self.session);
                crate::screenshots::private_dirs(&path, &root)?;
                let file = private_file(&path.join("events.jsonl"))?;
                s.disk = Some((path, file));
            }
            s.seq += 1;
            let mut meta = json!({"seq":s.seq,"timestamp_utc_epoch_ms":millis,"mcp_session_id":self.session,"tool":tool,"arguments":bounded(arguments),"native_action":native});
            if let Some(block) = frame
                .and_then(|f| f["content"].as_array())
                .and_then(|b| b.iter().find(|b| b["type"] == "image"))
            {
                let ext = if block["mimeType"] == "image/jpeg" {
                    "jpg"
                } else {
                    "png"
                };
                let name = format!("{}-{millis}.{ext}", s.seq);
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(block["data"].as_str().ok_or("missing frame data")?)?;
                private_file(&s.disk.as_ref().unwrap().0.join(&name))?.write_all(&bytes)?;
                meta["frame"] = json!(name);
            }
            let mut before = meta.clone();
            before["phase"] = json!("before");
            writeln!(s.disk.as_mut().unwrap().1, "{before}")?;
            s.pending += 1;
            Ok(Event {
                recorder: self.clone(),
                metadata: meta,
                finished: false,
            })
        })())
    }
    pub fn end(&self) {
        let mut s = self.state.lock().unwrap();
        s.ended = true;
        if s.pending == 0 {
            finalize(&mut s);
        }
    }
}
impl Event {
    pub fn finish(mut self, response: &Value) {
        self.metadata["response"] = bounded(response);
        self.metadata["phase"] = json!("after");
        let mut s = self.recorder.state.lock().unwrap();
        warn((|| {
            writeln!(s.disk.as_mut().unwrap().1, "{}", self.metadata)?;
            Ok(())
        })());
        self.finished = true;
        s.pending -= 1;
        if s.ended && s.pending == 0 {
            finalize(&mut s);
        }
    }
}
impl Drop for Event {
    fn drop(&mut self) {
        if !self.finished {
            // Cancellation cannot manufacture a completed platform response.
            self.metadata["phase"] = json!("after");
            self.metadata["response"] =
                json!({"cancelled":true,"outcome":"unknown; admitted native work may finish"});
            let recorder = self.recorder.clone();
            let metadata = self.metadata.clone();
            tokio::task::spawn_blocking(move || {
                let mut s = recorder.state.lock().unwrap();
                warn((|| {
                    writeln!(s.disk.as_mut().unwrap().1, "{metadata}")?;
                    Ok(())
                })());
                s.pending -= 1;
                if s.ended && s.pending == 0 {
                    finalize(&mut s);
                }
            });
        }
    }
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn finalize(s: &mut ReplayState) {
    let Some((path, log)) = s.disk.as_mut() else {
        return;
    };
    warn((|| {
        log.flush()?;
        let mut index = private_file(&path.join("index.html"))?;
        writeln!(index,"<!doctype html><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src 'self'\"><title>Local replay</title><h1>TINY-PROPOSAL local replay index</h1><p>Local frames and metadata. Not an AWS/Bedrock evidence report. No video.</p>")?;
        let mut reader = log.try_clone()?;
        reader.seek(SeekFrom::Start(0))?;
        for line in BufReader::new(reader).lines() {
            let v: Value = serde_json::from_str(&line?)?;
            if v["phase"] != "after" {
                continue;
            }
            writeln!(
                index,
                "<article><pre>{}</pre>",
                escape(&serde_json::to_string_pretty(&v)?)
            )?;
            if let Some(frame) = v["frame"].as_str() {
                writeln!(
                    index,
                    "<img alt=\"Pre-action frame\" src=\"{}\">",
                    escape(frame)
                )?;
            }
            writeln!(index, "</article>")?;
        }
        Ok(())
    })());
    s.disk = None;
}
impl Context {
    pub async fn begin(
        &self,
        native: &str,
        args: &Value,
        c: &crate::config::Config,
        p: Arc<dyn taa_platform::Platform>,
    ) -> Option<Event> {
        let frame = if native == "wait" {
            None
        } else {
            Some(crate::execute("screenshot", &json!({}), c, p).await)
        };
        if frame.as_ref().is_some_and(|f| f["isError"] == true) {
            eprintln!(
                "warning: replay pre-action capture failed: {}",
                frame.as_ref().unwrap()
            );
        }
        let context = self.clone();
        let native = native.to_owned();
        let args = args.clone();
        tokio::task::spawn_blocking(move || {
            let arguments = json!({"client":context.arguments,"native":args});
            context
                .recorder
                .begin(&context.tool, &arguments, Some(&native), frame.as_ref())
        })
        .await
        .ok()
        .flatten()
    }
}
pub async fn finish(event: Option<Event>, response: &Value) {
    if let Some(event) = event {
        let response = response.clone();
        let _ = tokio::task::spawn_blocking(move || event.finish(&response)).await;
    }
}
pub async fn execute(
    n: &str,
    a: &Value,
    c: &crate::config::Config,
    p: Arc<dyn taa_platform::Platform>,
    context: Option<&Context>,
) -> Value {
    let event = if let Some(context) = context {
        context.begin(n, a, c, p.clone()).await
    } else {
        None
    };
    let r = crate::execute(n, a, c, p).await;
    finish(event, &r).await;
    r
}
