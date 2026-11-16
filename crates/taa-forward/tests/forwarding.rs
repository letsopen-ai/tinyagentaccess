use serde_json::json;
use std::time::Duration;
use taa_forward::{forwarded_name, parse_config, rpc_error, ForwardingSet, Timeouts};
fn fixture(mode: &str, extra: &[&str]) -> (tempfile::TempDir, std::sync::Arc<ForwardingSet>) {
    fixture_with_limits(mode, extra, Timeouts::default())
}

fn fixture_with_limits(
    mode: &str,
    extra: &[&str],
    limits: Timeouts,
) -> (tempfile::TempDir, std::sync::Arc<ForwardingSet>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("forward.json");
    let args: Vec<_> = std::iter::once(mode).chain(extra.iter().copied()).collect();
    std::fs::write(&path, json!({"mcpServers":{"fixture":{"command":env!("CARGO_BIN_EXE_taa-fake-mcp-server"),"args":args}}}).to_string()).unwrap();
    (dir, ForwardingSet::new(path, limits))
}
#[test]
fn config_contract() {
    for bom in [
        &b"\xff\xfe\0\0"[..],
        &b"\0\0\xfe\xff"[..],
        &b"\xef\xbb\xbf"[..],
        &b"\xff\xfe"[..],
        &b"\xfe\xff"[..],
    ] {
        assert!(parse_config(bom).is_err());
    }
    for raw in [
        "{}",
        "null",
        "{",
        r#"{"mcpServers":[]}"#,
        r#"{"mcpServers":{"x":{}}}"#,
        r#"{"mcpServers":{"x":{"command":1}}}"#,
        r#"{"mcpServers":{"x":{"command":"x","args":null}}}"#,
        r#"{"mcpServers":{"x":{"command":"x","args":[1]}}}"#,
    ] {
        assert!(parse_config(raw.as_bytes()).is_err(), "{raw}");
    }
    assert!(parse_config(&[255]).is_err());
    let c = parse_config(br#"{"other":true,"mcpServers":{"z":{"command":"z","env":{},"cwd":"/"},"a":{"command":"a"}}}"#).unwrap();
    assert_eq!(c.warnings.len(), 3);
    assert_eq!(c.servers[0].name, "z");
    assert_eq!(forwarded_name("a___b", "c"), "forwarded___a___b___c");
    for code in [-32602, -32600, -32601] {
        assert_eq!(rpc_error(code, "bad"), "CLIENT_ERROR: bad");
    }
    assert_eq!(rpc_error(-32603, "bad"), "SERVER_ERROR: bad");
}
#[tokio::test]
async fn passthrough_and_parallel_calls() {
    let (_dir, set) = fixture("good", &[]);
    let tools = set.list().await;
    assert_eq!(tools.len(), 4);
    assert_eq!(tools[0]["annotations"]["readOnlyHint"], true);
    assert_eq!(tools[0]["outputSchema"]["type"], "object");
    assert!(tools
        .iter()
        .any(|t| t["name"] == "forwarded___fixture___odd___name"));
    let (a, b) = tokio::join!(
        set.call("forwarded___fixture___echo", json!({"text":"hello"})),
        set.call("forwarded___fixture___ping", json!({}))
    );
    assert_eq!(a["structuredContent"]["text"], "hello");
    assert_eq!(b["content"][0]["text"], "pong");
    assert_eq!(a["_meta"]["fixture"], true);
    assert_eq!(
        set.call("forwarded___fixture___odd___name", json!({}))
            .await["isError"],
        false
    );
    assert!(
        set.call("forwarded___fixture___echo", json!([])).await["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("CLIENT_ERROR:")
    );
    set.teardown().await;
    assert!(set.list().await.is_empty());
}
#[tokio::test]
async fn discovery_modes() {
    for (mode, count) in [
        ("empty", 0),
        ("paginate", 2),
        ("bad-stdout", 4),
        ("stderr-noise", 4),
        ("slow-list", 0),
        ("slow-init", 0),
    ] {
        if mode == "slow-init" {
            // Pay first-exec scanning costs before testing an initialize deadline.
            let (_warm_dir, warm) = fixture("good", &[]);
            assert_eq!(warm.list().await.len(), 4);
            warm.teardown().await;
        }
        // Only these modes test deadlines. Leave normal startup and requests
        // at production limits, including first-exec scanning on macOS.
        let limits = Timeouts {
            request: if mode == "slow-list" {
                Duration::from_millis(500)
            } else {
                Timeouts::default().request
            },
            initialize: if mode == "slow-init" {
                Duration::from_secs(5)
            } else {
                Timeouts::default().initialize
            },
            ..Timeouts::default()
        };
        let marker = tempfile::NamedTempFile::new().unwrap();
        let (_dir, set) = fixture_with_limits(
            mode,
            &["10", "--request-file", marker.path().to_str().unwrap()],
            limits,
        );
        if mode == "slow-init" {
            let listing = {
                let set = set.clone();
                tokio::spawn(async move { set.list().await })
            };
            // Observe initialize before cancellation overwrites the request marker.
            wait_for_request(marker.path(), "initialize").await;
            assert!(listing.await.unwrap().is_empty());
        } else {
            assert_eq!(set.list().await.len(), count, "{mode}");
        }
        set.teardown().await;
    }
}
#[tokio::test]
async fn timeout_sends_cancel() {
    let marker = tempfile::NamedTempFile::new().unwrap();
    let path = marker.path().to_str().unwrap();
    let (_dir, set) = fixture_with_limits(
        "slow-call",
        &["10", "--cancel-file", path],
        Timeouts {
            request: Duration::from_millis(500),
            ..Timeouts::default()
        },
    );
    // Complete startup/discovery before exercising the call deadline.
    assert_eq!(set.list().await.len(), 4);
    let result = set.call("forwarded___fixture___echo", json!({})).await;
    assert_eq!(
        result["content"][0]["text"],
        "SERVER_ERROR: forwarded tool call timed out after 5s"
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if std::fs::read_to_string(path)
                .unwrap()
                .contains("notifications/cancelled")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    set.teardown().await;
}
#[tokio::test]
async fn errors_crash_and_respawn() {
    let (_dir, set) = fixture("error-call", &[]);
    for (code, prefix) in [(-32602, "CLIENT_ERROR:"), (-32603, "SERVER_ERROR:")] {
        assert!(set
            .call("forwarded___fixture___echo", json!({"code":code}))
            .await["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with(prefix));
    }
    set.teardown().await;
    let (_dir, set) = fixture("tool-error", &[]);
    let r = set
        .call("forwarded___fixture___echo", json!({"text":"untouched"}))
        .await;
    assert_eq!(r["isError"], true);
    assert_eq!(r["content"][0]["text"], "untouched");
    set.teardown().await;
    let (_dir, set) = fixture("crash-on-call", &[]);
    assert_eq!(set.list().await.len(), 4);
    assert_eq!(
        set.call("forwarded___fixture___echo", json!({})).await["content"][0]["text"],
        "SERVER_ERROR: forwarded server 'fixture' is unavailable"
    );
    assert_eq!(set.list().await.len(), 4);
    set.teardown().await;
}
#[tokio::test]
async fn teardown_reaps_child() {
    let pid = tempfile::NamedTempFile::new().unwrap();
    let (_dir, set) = fixture("good", &["--pid-file", pid.path().to_str().unwrap()]);
    set.list().await;
    let pid: u32 = std::fs::read_to_string(pid.path())
        .unwrap()
        .parse()
        .unwrap();
    set.teardown().await;
    // Portable process probe: no Unix-only cargo test dependencies.
    #[cfg(unix)]
    assert!(!std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success());
    #[cfg(windows)]
    assert!(!String::from_utf8_lossy(
        &std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .unwrap()
            .stdout
    )
    .contains("taa-fake"));
}
#[tokio::test]
async fn teardown_interrupts_hung_initialize() {
    let marker = tempfile::NamedTempFile::new().unwrap();
    let (_dir, set) = fixture(
        "slow-init",
        &["120", "--request-file", marker.path().to_str().unwrap()],
    );
    let listing = {
        let set = set.clone();
        tokio::spawn(async move { set.list().await })
    };
    wait_for_request(marker.path(), "initialize").await;
    // Startup has completed. Teardown must interrupt initialize well before
    // its 60s deadline (and the fixture's 120s response).
    tokio::time::timeout(Duration::from_secs(5), async {
        set.teardown().await;
        assert!(listing.await.unwrap().is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn warm_retry_and_list_cancel() {
    let marker = tempfile::NamedTempFile::new().unwrap();
    let (_dir, set) = fixture_with_limits(
        "slow-list",
        &[
            "10",
            "--warm-retry",
            "--cancel-file",
            marker.path().to_str().unwrap(),
        ],
        Timeouts {
            request: Duration::from_millis(500),
            ..Timeouts::default()
        },
    );
    assert!(set.list().await.is_empty());
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if std::fs::read_to_string(marker.path())
                .unwrap()
                .contains("notifications/cancelled")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(set.list().await.len(), 4);
    assert_eq!(
        set.call("forwarded___fixture___echo", json!({"text":"warm"}))
            .await["content"][0]["text"],
        "warm"
    );
    set.teardown().await;
}
#[tokio::test]
async fn collision_uses_configuration_order_and_exact_names() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("redirect.json");
    let exe = serde_json::to_string(env!("CARGO_BIN_EXE_taa-fake-mcp-server")).unwrap();
    // Both servers produce forwarded___fixture___odd___name. First in JSON
    // order wins even though lexicographic map order would choose fixture first.
    std::fs::write(&path, format!(r#"{{"mcpServers":{{"fixture___odd":{{"command":{exe},"args":["tool-error","--single-tool","name"]}},"fixture":{{"command":{exe},"args":["good"]}}}}}}"#)).unwrap();
    let set = ForwardingSet::new(path, Timeouts::default());
    let tools = set.list().await;
    assert_eq!(tools.len(), 4);
    assert_eq!(tools[0]["name"], "forwarded___fixture___odd___name");
    assert_eq!(
        set.call("forwarded___fixture___odd___name", json!({}))
            .await["isError"],
        true
    );
    assert_eq!(
        set.call("forwarded___fixture___echo", json!({})).await["isError"],
        false
    );
    set.teardown().await;
}
#[tokio::test]
async fn teardown_kills_uncooperative_child() {
    let marker = tempfile::NamedTempFile::new().unwrap();
    let (_dir, set) = fixture(
        "good",
        &[
            "--pid-file",
            marker.path().to_str().unwrap(),
            "--linger-on-eof",
        ],
    );
    set.list().await;
    let pid: u32 = std::fs::read_to_string(marker.path())
        .unwrap()
        .parse()
        .unwrap();
    set.teardown().await;
    #[cfg(unix)]
    assert!(!std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success());
    #[cfg(windows)]
    assert!(!String::from_utf8_lossy(
        &std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .unwrap()
            .stdout
    )
    .contains("taa-fake"));
}

async fn wait_for_request(path: &std::path::Path, method: &str) {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if std::fs::read_to_string(path).unwrap_or_default() == method {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn healthy_calls_during_slow_discovery() {
    for mode in ["slow-init", "slow-list"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let marker = dir.path().join("request");
        let exe = env!("CARGO_BIN_EXE_taa-fake-mcp-server");
        std::fs::write(
            &path,
            json!({"mcpServers":{
                "healthy":{"command":exe,"args":["good"]},
                "slow":{"command":exe,"args":[mode,"10","--request-file",marker]}
            }})
            .to_string(),
        )
        .unwrap();
        let limits = Timeouts {
            request: Duration::from_millis(500),
            initialize: Duration::from_secs(5),
            graceful: Duration::from_millis(100),
            teardown: Duration::from_secs(1),
        };
        let set = ForwardingSet::new(path, limits);
        assert_eq!(set.list().await.len(), 4);
        std::fs::remove_file(&marker).unwrap();
        let refresh = {
            let set = set.clone();
            tokio::spawn(async move { set.list().await })
        };
        wait_for_request(
            &marker,
            if mode == "slow-init" {
                "initialize"
            } else {
                "tools/list"
            },
        )
        .await;
        let result = tokio::time::timeout(
            Duration::from_millis(200),
            set.call("forwarded___healthy___ping", json!({})),
        )
        .await
        .unwrap();
        assert_eq!(result["content"][0]["text"], "pong");
        set.teardown().await;
        assert!(refresh.await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn teardown_during_initialize_reaps_unpublished_child() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let marker = dir.path().join("request");
    let pid_path = dir.path().join("pid");
    std::fs::write(
        &path,
        json!({"mcpServers":{"slow":{
            "command":env!("CARGO_BIN_EXE_taa-fake-mcp-server"),
            "args":["slow-init","60","--pid-file",pid_path,"--request-file",marker]
        }}})
        .to_string(),
    )
    .unwrap();
    let set = ForwardingSet::new(path, Timeouts::default());
    let refresh = {
        let set = set.clone();
        tokio::spawn(async move { set.list().await })
    };
    wait_for_request(&marker, "initialize").await;
    let pid = std::fs::read_to_string(pid_path).unwrap();
    tokio::time::timeout(Duration::from_millis(500), set.teardown())
        .await
        .unwrap();
    assert!(refresh.await.unwrap().is_empty());
    assert!(set.list().await.is_empty());
    #[cfg(unix)]
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if !std::process::Command::new("kill")
                .args(["-0", &pid])
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .success()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    #[cfg(windows)]
    assert!(!String::from_utf8_lossy(
        &std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .unwrap()
            .stdout
    )
    .contains("taa-fake"));
}
