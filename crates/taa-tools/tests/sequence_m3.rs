use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use taa_platform::{Frame, MockPlatform, Operation, Platform};
use taa_tools::{config::Config, sequence_with_limits, SequenceLimits};
fn body(r: &Value) -> Value {
    serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap()
}
fn limits() -> SequenceLimits {
    SequenceLimits {
        deadline: Duration::from_secs(60),
        stabilization: Duration::ZERO,
    }
}
async fn run(actions: Value) -> Value {
    sequence_with_limits(
        &json!({"actions":actions}),
        &Config::default(),
        Arc::new(MockPlatform::recording()),
        limits(),
    )
    .await
}
fn rejected(r: &Value, detail: &str, count: usize) {
    assert_eq!(r["isError"], true);
    assert_eq!(r["content"].as_array().unwrap().len(), 1);
    let b = body(r);
    assert_eq!(
        b["sequence_summary"]["stop_reason"],
        format!("CLIENT_ERROR: Rejected before dispatch: {detail}")
    );
    assert_eq!(
        b["sequence_summary"]["guidance"],
        "Correct the sequence and retry. No desktop action ran, so no screenshot is needed."
    );
    assert_eq!(b["sequence_summary"]["executed"], 0);
    assert_eq!(b["sequence_summary"]["succeeded"], 0);
    assert_eq!(b["sequence_summary"]["submitted"], count);
    for row in b["action_results"].as_array().unwrap() {
        assert_eq!(row["status"], "skipped");
        assert_eq!(row["reason"], "sequence rejected before dispatch");
    }
}
#[tokio::test]
async fn exact_rejections_and_validation() {
    rejected(
        &run(json!(vec![json!({"type":"key","keys":"a"}); 21])).await,
        "too many actions: 21 > 20",
        21,
    );
    for duration in [json!(0.0009), json!(5.0001), json!("1"), Value::Null] {
        rejected(
            &run(json!([{"type":"wait","duration":duration}])).await,
            "action 0: wait duration must be a number between 0.001 and 5.0 seconds",
            1,
        );
    }
    rejected(
        &run(json!([{"type":"wait"}])).await,
        "action 0: wait duration must be a number between 0.001 and 5.0 seconds",
        1,
    );
    rejected(
        &run(json!(vec![json!({"type":"wait","duration":5}); 3])).await,
        "cumulative wait 15.0s exceeds 10.0s",
        3,
    );
    rejected(
        &run(json!(vec![
            json!({"type":"type_text","text":"中".repeat(5500)});
            2
        ]))
        .await,
        "cumulative type_text length 11000 exceeds 10000",
        2,
    );
    rejected(&run(json!([{"type":"type_text","text":"😀".repeat(5000)},{"type":"type_text","text":"中".repeat(5001)}])).await,"cumulative type_text length 10001 exceeds 10000",2);
    for n in ["screenshot", "execute_sequence"] {
        rejected(
            &run(json!([{"type":format!("agentaccess___{n}")}])).await,
            &format!("action 0: {n} cannot be nested in execute_sequence"),
            1,
        );
    }
    for n in [
        "missing",
        "computer",
        "forwarded___x___key",
        "get_session_info",
        "close_session",
    ] {
        rejected(&run(json!([{"type":n}])).await, "action 0: Unknown tool", 1);
    }
    rejected(
        &run(json!([{"type":"key","keys":"bad_key"}])).await,
        &format!(
            "action 0: {}",
            taa_tools::validate("key", &json!({"keys":"bad_key"}), (1280, 720)).unwrap_err()
        ),
        1,
    );
    rejected(&run(json!([])).await, "actions must not be empty", 0);
    for args in [json!({}), json!({"actions":4})] {
        let r = sequence_with_limits(
            &args,
            &Config::default(),
            Arc::new(MockPlatform::recording()),
            limits(),
        )
        .await;
        rejected(&r, "actions must be an array", 0);
    }
}
#[tokio::test]
async fn boundaries_and_success_shape() {
    for actions in [
        json!(vec![json!({"type":"agentaccess___key","keys":"a"}); 20]),
        json!([{"type":"type_text","text":"😀".repeat(5000)},{"type":"type_text","text":"中".repeat(5000)}]),
    ] {
        let r = run(actions).await;
        assert_eq!(r["isError"], false);
        let b = body(&r);
        assert_eq!(
            b["sequence_summary"]["executed"],
            b["sequence_summary"]["submitted"]
        );
        assert_eq!(
            b["sequence_summary"]["succeeded"],
            b["sequence_summary"]["submitted"]
        );
        assert_eq!(b["sequence_summary"]["guidance"], "");
        for row in b["action_results"].as_array().unwrap() {
            assert_eq!(row["status"], "success");
            assert!(row["duration_ms"].is_number());
        }
        assert_eq!(b["terminal_screenshot"]["status"], "success");
        assert!(b["terminal_screenshot"]["duration_ms"].is_number());
        assert_eq!(r["content"][1]["type"], "image");
    }
}
#[tokio::test(start_paused = true)]
async fn wait_boundaries_and_pause_skip() {
    // Synthetic small native image avoids runtime worker CPU affecting paused-clock assertions.
    for actions in [
        json!([{"type":"wait","duration":0.001}]),
        json!([{"type":"wait","duration":5},{"type":"wait","duration":5}]),
    ] {
        let mut c = Config::default();
        c.agent_access.computer_vision = false;
        let start = tokio::time::Instant::now();
        let r = sequence_with_limits(
            &json!({"actions":actions}),
            &c,
            Arc::new(MockPlatform::recording()),
            SequenceLimits {
                deadline: Duration::from_secs(11),
                stabilization: Duration::from_secs(30),
            },
        )
        .await;
        assert_eq!(r["isError"], false);
        assert!(start.elapsed() <= Duration::from_secs(10));
        assert_eq!(body(&r)["terminal_screenshot"]["status"], "skipped");
    }
}
#[tokio::test(start_paused = true)]
async fn deadlines_clamp_wait_and_pause() {
    let start = tokio::time::Instant::now();
    let r = sequence_with_limits(
        &json!({"actions":[{"type":"wait","duration":5},{"type":"key","keys":"a"}]}),
        &Config::default(),
        Arc::new(MockPlatform::recording()),
        SequenceLimits {
            deadline: Duration::from_millis(10),
            stabilization: Duration::ZERO,
        },
    )
    .await;
    assert_eq!(start.elapsed(), Duration::from_millis(10));
    assert_eq!(r["isError"], true);
    assert_eq!(r["content"].as_array().unwrap().len(), 1);
    let b = body(&r);
    assert_eq!(
        b["sequence_summary"]["stop_reason"],
        "Deadline reached before action 1"
    );
    assert_eq!(b["action_results"][1]["status"], "skipped");
    let r = sequence_with_limits(
        &json!({"actions":[{"type":"wait","duration":0.001}]}),
        &Config::default(),
        Arc::new(MockPlatform::recording()),
        SequenceLimits {
            deadline: Duration::ZERO,
            stabilization: Duration::ZERO,
        },
    )
    .await;
    assert_eq!(
        body(&r)["sequence_summary"]["stop_reason"],
        "Deadline reached before action 0"
    );
}
struct Failing {
    calls: Mutex<usize>,
    screenshot_fails: bool,
}
impl Platform for Failing {
    fn name(&self) -> &str {
        "failure"
    }
    fn screen_size(&self) -> (u32, u32) {
        (2, 2)
    }
    fn capture(&self) -> Result<Frame, String> {
        if self.screenshot_fails {
            Err("capture failed".into())
        } else {
            Ok(Frame::solid(2, 2, [0, 0, 0, 255]))
        }
    }
    fn perform(&self, _: Operation) -> Result<(), String> {
        let mut n = self.calls.lock().unwrap();
        *n += 1;
        if *n == 2 {
            Err("injected failure".into())
        } else {
            Ok(())
        }
    }
}
#[tokio::test]
async fn stops_on_failure_and_capture_error() {
    let p = Arc::new(Failing {
        calls: Mutex::new(0),
        screenshot_fails: false,
    });
    let r = sequence_with_limits(
        &json!({"actions":vec![json!({"type":"key","keys":"a"});3]}),
        &Config::default(),
        p.clone(),
        limits(),
    )
    .await;
    assert_eq!(*p.calls.lock().unwrap(), 2);
    assert_eq!(r["isError"], true);
    assert_eq!(r["content"].as_array().unwrap().len(), 1);
    let b = body(&r);
    assert_eq!(b["sequence_summary"]["executed"], 2);
    assert_eq!(b["sequence_summary"]["succeeded"], 1);
    assert_eq!(b["action_results"][1]["status"], "error");
    assert_eq!(b["action_results"][2]["status"], "skipped");
    let p = Arc::new(Failing {
        calls: Mutex::new(0),
        screenshot_fails: true,
    });
    let r = sequence_with_limits(
        &json!({"actions":[{"type":"key","keys":"a"}]}),
        &Config::default(),
        p,
        limits(),
    )
    .await;
    assert_eq!(r["isError"], true);
    assert_eq!(body(&r)["terminal_screenshot"]["status"], "error");
}
#[tokio::test]
async fn config_gating() {
    for vision in [false, true] {
        let mut c = Config::default();
        c.agent_access.computer_input = false;
        c.agent_access.computer_vision = vision;
        assert!(!taa_tools::visible("execute_sequence", &c, false));
        let p = Arc::new(MockPlatform::recording());
        let r = sequence_with_limits(
            &json!({"actions":[{"type":"key","keys":"a"}]}),
            &c,
            p.clone(),
            limits(),
        )
        .await;
        rejected(&r, "action 0: Unknown tool", 1);
        assert!(p.calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn stabilization_is_subject_to_deadline() {
    let mut c = Config::default();
    c.agent_access.computer_vision = false;
    let r = sequence_with_limits(
        &json!({"actions":[{"type":"key","keys":"a"}]}),
        &c,
        Arc::new(MockPlatform::recording()),
        SequenceLimits {
            deadline: Duration::from_millis(25),
            stabilization: Duration::from_secs(1),
        },
    )
    .await;
    assert_eq!(r["isError"], true);
    assert_eq!(r["content"].as_array().unwrap().len(), 1);
    assert!(body(&r)["sequence_summary"]["stop_reason"]
        .as_str()
        .unwrap()
        .starts_with("Deadline reached"));
}
