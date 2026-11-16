use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use taa_platform::{Frame, Operation, Platform};
use taa_tools::{
    config::Config,
    replay::{self, Context, Recorder},
    SequenceLimits,
};
fn files(path: &Path) -> Vec<PathBuf> {
    fs::read_dir(path)
        .unwrap()
        .flat_map(|e| {
            let p = e.unwrap().path();
            if p.is_dir() {
                files(&p)
            } else {
                vec![p]
            }
        })
        .collect()
}
struct Ordered {
    events: Mutex<Vec<&'static str>>,
    fail: bool,
    root: PathBuf,
}
impl Platform for Ordered {
    fn name(&self) -> &str {
        "ordered"
    }
    fn screen_size(&self) -> (u32, u32) {
        (10, 10)
    }
    fn capture(&self) -> Result<Frame, String> {
        self.events.lock().unwrap().push("capture");
        Ok(Frame::solid(10, 10, [0, 0, 0, 255]))
    }
    fn perform(&self, _: Operation) -> Result<(), String> {
        self.events.lock().unwrap().push("action");
        // The before record and frame must be published before native dispatch.
        let log = files(&self.root)
            .into_iter()
            .find(|p| p.ends_with("events.jsonl"))
            .unwrap();
        let rows: Vec<Value> = fs::read_to_string(log)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.last().unwrap()["phase"], "before");
        assert!(rows.last().unwrap()["frame"].is_string());
        if self.fail {
            Err("injected failure".into())
        } else {
            Ok(())
        }
    }
}
fn context(root: &Path) -> Context {
    Context {
        recorder: Recorder::new(root.to_str().unwrap().into(), "session-1".into()),
        tool: "computer".into(),
        arguments: json!({"action":"key","text":"a"}),
    }
}
#[test]
fn config_validation() {
    assert!(!Config::parse("").unwrap().replay.enabled);
    for text in [
        "[replay]\nenabled=true",
        "[replay]\nenabled=true\nsave_dir='relative'",
        "[replay]\nunknown=true",
        "[agent_access]\nuser_control_mode='invalid'",
    ] {
        assert!(Config::parse(text).is_err(), "{text}");
    }
    for mode in ["disabled", "view_only", "view_stop"] {
        assert!(Config::parse(&format!("[agent_access]\nuser_control_mode='{mode}'")).is_ok());
    }
    let tmp = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.replay.enabled = true;
    config.replay.save_dir = tmp.path().to_str().unwrap().into();
    assert!(config.validate().is_ok());
}
#[tokio::test]
async fn before_frame_then_response_success_and_failure() {
    for fail in [false, true] {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = context(tmp.path());
        let p = Arc::new(Ordered {
            events: Mutex::new(vec![]),
            fail,
            root: tmp.path().into(),
        });
        let mut c = Config::default();
        c.agent_access.screen_resolution = "native".into();
        let r = replay::execute("key", &json!({"keys":"a"}), &c, p.clone(), Some(&ctx)).await;
        assert_eq!(r["isError"], fail);
        assert_eq!(*p.events.lock().unwrap(), vec!["capture", "action"]);
        ctx.recorder.end();
        let all = files(tmp.path());
        let log = all.iter().find(|p| p.ends_with("events.jsonl")).unwrap();
        let rows: Vec<Value> = fs::read_to_string(log)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["tool"], "computer");
        assert_eq!(rows[0]["native_action"], "key");
        assert_eq!(rows[1]["response"], r);
        assert!(rows[0]["timestamp_utc_epoch_ms"].is_u64());
        assert_eq!(rows[0]["mcp_session_id"], "session-1");
        assert!(log
            .parent()
            .unwrap()
            .join(rows[0]["frame"].as_str().unwrap())
            .exists());
        let index = all.iter().find(|p| p.ends_with("index.html")).unwrap();
        assert!(fs::read_to_string(index).unwrap().contains("TINY-PROPOSAL"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for p in &all {
                assert_eq!(fs::metadata(p).unwrap().permissions().mode() & 0o777, 0o600);
            }
            assert_eq!(
                fs::metadata(log.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
    }
}
#[tokio::test]
async fn sequence_steps_wait_and_disabled_and_io_failures() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = context(tmp.path());
    let p = Arc::new(taa_platform::MockPlatform::recording());
    let mut c = Config::default();
    c.agent_access.screen_resolution = "native".into();
    let r=taa_tools::sequence_recorded(&json!({"actions":[{"type":"key","keys":"a"},{"type":"wait","duration":0.001},{"type":"key","keys":"b"}]}),&c,p.clone(),SequenceLimits{stabilization:std::time::Duration::ZERO,..Default::default()},Some(&ctx)).await;
    assert_eq!(r["isError"], false);
    ctx.recorder.end();
    let all = files(tmp.path());
    let log = all.iter().find(|p| p.ends_with("events.jsonl")).unwrap();
    let rows: Vec<Value> = fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(rows.len(), 6);
    assert_eq!(rows[2]["native_action"], "wait");
    assert!(rows[2].get("frame").is_none());
    let empty = tempfile::tempdir().unwrap();
    let r = replay::execute("key", &json!({"keys":"a"}), &c, p.clone(), None).await;
    assert_eq!(r["isError"], false);
    assert!(files(empty.path()).is_empty());
    let blocked = empty.path().join("blocked");
    fs::write(&blocked, b"file").unwrap();
    let bad = context(&blocked);
    let r = replay::execute("key", &json!({"keys":"a"}), &c, p, Some(&bad)).await;
    assert_eq!(r["isError"], false);
}
#[cfg(unix)]
#[test]
fn refuses_symlinks_and_unsafe_session_and_existing_files() {
    let tmp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), tmp.path().join("agentaccess")).unwrap();
    let ctx = context(tmp.path());
    assert!(ctx.recorder.begin("key", &json!({}), None, None).is_none());
    assert!(files(outside.path()).is_empty());
    let r = Recorder::new(outside.path().to_str().unwrap().into(), "../escape".into());
    assert!(r.begin("key", &json!({}), None, None).is_none());
    let ctx = context(outside.path());
    ctx.recorder
        .begin("info", &json!({}), None, None)
        .unwrap()
        .finish(&json!({"ok":true}));
    ctx.recorder.end();
    let other = context(outside.path());
    assert!(other
        .recorder
        .begin("info", &json!({}), None, None)
        .is_none());
}
