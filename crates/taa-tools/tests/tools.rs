use serde_json::{json, Value};
use std::sync::Arc;
use taa_platform::MockPlatform;
use taa_tools::{config::Config, *};
#[test]
fn config() {
    assert!(Config::parse("").is_ok());
    assert!(Config::parse("typo = 1").is_err());
    assert!(Config::parse("[agent_access]\ncomputer_vision=false").is_err());
    assert!(Config::parse("[sessions]\nmax_sessions=0").is_err());
}
#[test]
fn catalogs() {
    for modern in [false, true] {
        for (vision, input) in [(true, true), (true, false), (false, false)] {
            let mut c = Config::default();
            c.agent_access.computer_vision = vision;
            c.agent_access.computer_input = input;
            let r = list(&c, modern);
            let tools = r["tools"].as_array().unwrap();
            assert_eq!(
                tools.len(),
                2 + usize::from(modern) + usize::from(vision) + if input { 14 } else { 0 }
            );
            for t in tools {
                let n = t["name"].as_str().unwrap().strip_prefix(PREFIX).unwrap();
                assert!(NAMES.contains(&n));
                assert_eq!(
                    t["inputSchema"]["required"]
                        .as_array()
                        .unwrap()
                        .contains(&json!("mcpSessionId")),
                    modern
                );
            }
        }
    }
    assert_eq!(
        schema("wait", false)["properties"]["duration"]["minimum"],
        0.001
    );
    assert_eq!(
        schema("wait", false)["properties"]["duration"]["maximum"],
        5.0
    );
    assert_eq!(
        schema("execute_sequence", false)["properties"]["actions"]["maxItems"],
        20
    );
    assert_eq!(
        schema("type_text", false)["properties"]["text"]["maxLength"],
        10000
    );
}
#[tokio::test]
async fn waits() {
    let c = Config::default();
    let p = Arc::new(MockPlatform::default());
    for d in [
        json!(0),
        json!(0.0005),
        json!(5.0001),
        json!("1"),
        Value::Null,
    ] {
        let r = execute("wait", &json!({"duration":d}), &c, p.clone()).await;
        assert_eq!(r["isError"], true);
        assert!(r["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("CLIENT_ERROR:"));
    }
    assert_eq!(
        execute("wait", &json!({}), &c, p.clone()).await["isError"],
        true
    );
    for d in [0.001, 0.002] {
        assert_eq!(
            execute("wait", &json!({"duration":d}), &c, p.clone()).await["isError"],
            false
        );
    }
}
#[tokio::test]
async fn routing_and_bounds() {
    let c = Config::default();
    let p = Arc::new(MockPlatform::recording());
    for (n, a) in [
        ("left_click", json!({"x":1,"y":2,"modifiers":"ctrl"})),
        (
            "left_click_drag",
            json!({"start_x":1,"start_y":2,"end_x":3,"end_y":4}),
        ),
        (
            "scroll",
            json!({"x":1,"y":2,"scroll_direction":"Down","scroll_amount":120}),
        ),
        ("key", json!({"keys":"ctrl+c"})),
        ("hold_key", json!({"keys":"shift","duration":1})),
        ("type_text", json!({"text":"hello"})),
    ] {
        assert_eq!(execute(n, &a, &c, p.clone()).await["isError"], false);
    }
    assert_eq!(p.calls.lock().unwrap().len(), 6);
    assert_eq!(p.calls.lock().unwrap()[0].0, "click:left:1");
    assert!(validate("move_pointer", &json!({"x":1280,"y":0}), (1280, 720)).is_err());
    assert!(validate("move_pointer", &json!({"x":1.5,"y":0}), (1280, 720)).is_err());
    assert_eq!(
        execute(
            "screenshot",
            &json!({}),
            &c,
            Arc::new(MockPlatform::default())
        )
        .await["content"][0]["text"],
        "SERVER_ERROR: desktop backend not available on this platform yet"
    );
}
#[tokio::test]
async fn rejections() {
    let c = Config::default();
    let p = Arc::new(MockPlatform::recording());
    for (actions, expected) in [
        (
            vec![json!({"type":"key","keys":"a"}); 21],
            "too many actions: 21 > 20".to_string(),
        ),
        (
            vec![json!({"type":"wait","duration":0})],
            "action 0: wait duration must be a number between 0.001 and 5.0 seconds".into(),
        ),
        (
            vec![json!({"type":"wait","duration":5}); 3],
            "cumulative wait 15.0s exceeds 10.0s".into(),
        ),
        (
            vec![json!({"type":"type_text","text":"x".repeat(5500)}); 2],
            "cumulative type_text length 11000 exceeds 10000".into(),
        ),
        (
            vec![json!({"type":"screenshot"})],
            "action 0: action cannot be nested or is unavailable".into(),
        ),
    ] {
        let r = sequence(&json!({"actions":actions}), &c, p.clone()).await;
        assert_eq!(r["isError"], true);
        let body: Value = serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(body["sequence_summary"]["stop_reason"], expected);
        assert_eq!(body["sequence_summary"]["executed"], 0);
        assert_eq!(
            body["action_results"][0]["reason"],
            "sequence rejected before dispatch"
        );
    }
    assert!(p.calls.lock().unwrap().is_empty());
}
#[tokio::test]
async fn sequence_dispatch() {
    let c = Config::default();
    let p = Arc::new(MockPlatform::recording());
    let r=sequence(&json!({"actions":[{"type":"agentaccess___key","keys":"Return"},{"type":"wait","duration":0.001}]}),&c,p.clone()).await;
    assert_eq!(r["isError"], false);
    assert_eq!(p.calls.lock().unwrap().len(), 2);
}

#[test]
fn integer_coordinates_are_intentionally_strict() {
    assert!(taa_tools::validate("move_pointer", &json!({"x":2,"y":3}), (1280, 720)).is_ok());
    assert!(taa_tools::validate("move_pointer", &json!({"x":2.0,"y":3}), (1280, 720)).is_err());
}

#[tokio::test]
async fn typed_mapping_and_encoding() {
    use base64::Engine;
    use taa_platform::{
        pixels::{decode_png, Point},
        Button, Key, Operation,
    };
    let c = Config::default();
    let p = Arc::new(MockPlatform::recording());
    let r = execute(
        "left_click",
        &json!({"x":640,"y":360,"modifiers":"ctrl"}),
        &c,
        p.clone(),
    )
    .await;
    assert_eq!(r["isError"], false);
    assert_eq!(
        p.operations.lock().unwrap()[0],
        Operation::Click(Point { x: 960, y: 540 }, Button::Left, 1, vec![Key(0xffe3)])
    );
    let r = execute("screenshot", &json!({}), &c, p.clone()).await;
    assert_eq!(r["content"][0]["mimeType"], "image/png");
    let b = base64::engine::general_purpose::STANDARD
        .decode(r["content"][0]["data"].as_str().unwrap())
        .unwrap();
    let f = decode_png(&b).unwrap();
    assert_eq!((f.width, f.height), (1280, 720));
    assert_eq!(&f.rgba[..4], &[40, 80, 120, 255]);
    for args in [json!({"keys":"ctrl+not_a_key"}), json!({"keys":"F25"})] {
        assert!(
            execute("key", &args, &c, p.clone()).await["content"][0]["text"]
                .as_str()
                .unwrap()
                .starts_with("CLIENT_ERROR:")
        );
    }
    assert!(validate(
        "left_click",
        &json!({"x":0,"y":0,"modifiers":"ctrl+a"}),
        (1280, 720)
    )
    .is_err());
    let mut c = c;
    c.agent_access.screen_resolution = "native".into();
    assert_eq!(screen(&c, p.as_ref()), (1920, 1080));
    assert_eq!(
        execute("left_click", &json!({"x":1919,"y":1079}), &c, p.clone()).await["isError"],
        false
    );
    assert_eq!(
        p.operations.lock().unwrap().last().unwrap(),
        &Operation::Click(Point { x: 1919, y: 1079 }, Button::Left, 1, vec![])
    );
}
