use serde_json::{json, Value};
use std::sync::Arc;
use taa_platform::MockPlatform;
use taa_tools::{
    config::Config,
    dialects::{self, Interface},
    execute,
};
const IDS: &[&str] = &[
    "ANTHROPIC_COMPUTER_20250124",
    "ANTHROPIC_COMPUTER_20251124",
    "ANTHROPIC_COMPUTER_TOOLSET_20260801",
    "OPENAI_COMPUTER_V1",
    "GOOGLE_COMPUTER_USE_3X_DESKTOP_V1",
];
fn interface(id: &str) -> Interface {
    Interface::parse(id).unwrap().unwrap()
}
#[test]
fn catalogs() {
    for id in IDS {
        for modern in [false, true] {
            let r = dialects::list(Some(interface(id)), &Config::default(), modern);
            let native = taa_tools::list(&Config::default(), modern);
            for tool in native["tools"].as_array().unwrap() {
                if dialects::retained(tool["name"].as_str().unwrap()) {
                    assert!(r["tools"].as_array().unwrap().contains(tool));
                }
            }
            let names = r["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["name"].as_str().unwrap())
                .collect::<Vec<_>>();
            for name in [
                "agentaccess___wait",
                "agentaccess___execute_sequence",
                "agentaccess___get_session_info",
            ] {
                assert!(names.contains(&name));
            }
            assert_eq!(names.contains(&"agentaccess___close_session"), modern);
            assert!(!names.contains(&"agentaccess___type_text"));
            for name in ["wait", "zoom", "cursor_position"] {
                assert!(!names.contains(&name));
            }
            let count = if id.contains("TOOLSET") || id.starts_with("GOOGLE") {
                14
            } else {
                1
            };
            assert_eq!(names.len(), count + 3 + usize::from(modern));
            for tool in r["tools"].as_array().unwrap() {
                assert_eq!(
                    tool["inputSchema"]["required"]
                        .as_array()
                        .unwrap()
                        .contains(&json!("mcpSessionId")),
                    modern
                );
            }
        }
    }
    assert!(dialects::retained("agentaccess___launch_application"));
    assert!(dialects::retained("agentaccess___switch_user"));
    assert!(Interface::parse("openai_computer_v1").is_err());
}
async fn mapped(i: Interface, name: &str, args: Value, native: &str, expected: Value) {
    let (n, a) = i.translate(name, &args, (1280, 720)).unwrap();
    assert_eq!(n, native);
    assert_eq!(a, expected);
    let p = Arc::new(MockPlatform::recording());
    let q = Arc::new(MockPlatform::recording());
    let c = Config::default();
    let actual = execute(&n, &a, &c, p.clone()).await;
    let expected = execute(native, &expected, &c, q.clone()).await;
    assert_eq!(actual, expected);
    assert_eq!(actual["isError"], false, "{actual}");
    assert_eq!(*p.operations.lock().unwrap(), *q.operations.lock().unwrap());
}
#[tokio::test]
async fn anthropic_actions() {
    for id in &IDS[..3] {
        let i = interface(id);
        let mut cases = vec![
            ("screenshot", json!({}), "screenshot", json!({})),
            (
                "type",
                json!({"text":"hello"}),
                "type_text",
                json!({"text":"hello"}),
            ),
            (
                "key",
                json!({"text":"Control+Return"}),
                "key",
                json!({"keys":"ctrl+Return"}),
            ),
            (
                "hold_key",
                json!({"text":"Shift","duration":1}),
                "hold_key",
                json!({"keys":"shift","duration":1}),
            ),
            (
                "mouse_move",
                json!({"coordinate":[3,4]}),
                "move_pointer",
                json!({"x":3,"y":4}),
            ),
            (
                "left_click_drag",
                json!({"start_coordinate":[1,2],"coordinate":[3,4]}),
                "left_click_drag",
                json!({"start_x":1,"start_y":2,"end_x":3,"end_y":4}),
            ),
            (
                "scroll",
                json!({"coordinate":[3,4],"scroll_direction":"down","scroll_amount":2,"text":"ctrl"}),
                "scroll",
                json!({"x":3,"y":4,"scroll_direction":"Down","scroll_amount":240,"modifiers":"ctrl"}),
            ),
        ];
        for name in [
            "left_click",
            "right_click",
            "middle_click",
            "double_click",
            "triple_click",
            "left_mouse_down",
            "left_mouse_up",
        ] {
            cases.push((
                name,
                json!({"coordinate":[3,4],"text":"ctrl"}),
                name,
                json!({"x":3,"y":4,"modifiers":"ctrl"}),
            ));
        }
        for (action, mut args, native, expected) in cases {
            let name = if id.contains("TOOLSET") {
                action
            } else {
                args["action"] = json!(action);
                "computer"
            };
            mapped(i, name, args, native, expected).await;
        }
        for action in ["wait", "zoom", "cursor_position"] {
            let (name, args) = if id.contains("TOOLSET") {
                (action, json!({}))
            } else {
                ("computer", json!({"action":action,"duration":1}))
            };
            assert!(i
                .translate(name, &args, (1280, 720))
                .unwrap_err()
                .contains("agentaccess___wait"));
        }
    }
}
#[tokio::test]
async fn openai_actions() {
    let i = interface(IDS[3]);
    for (action, expected) in [
        ("left", "left_click"),
        ("right", "right_click"),
        ("middle", "middle_click"),
    ] {
        mapped(
            i,
            "computer",
            json!({"type":"click","button":action,"x":3,"y":4}),
            expected,
            json!({"x":3,"y":4}),
        )
        .await;
    }
    for (args, native, expected) in [
        (
            json!({"type":"click","x":3,"y":4}),
            "left_click",
            json!({"x":3,"y":4}),
        ),
        (
            json!({"type":"double_click","x":3,"y":4}),
            "double_click",
            json!({"x":3,"y":4}),
        ),
        (
            json!({"type":"move","x":3,"y":4}),
            "move_pointer",
            json!({"x":3,"y":4}),
        ),
        (
            json!({"type":"drag","path":[{"x":1,"y":2},{"x":2,"y":3},{"x":3,"y":4}]}),
            "left_click_drag",
            json!({"start_x":1,"start_y":2,"end_x":3,"end_y":4}),
        ),
        (
            json!({"type":"scroll","x":3,"y":4,"scroll_x":0,"scroll_y":-120}),
            "scroll",
            json!({"x":3,"y":4,"scroll_direction":"Up","scroll_amount":120}),
        ),
        (
            json!({"type":"keypress","keys":["CTRL","SHIFT","a"]}),
            "key",
            json!({"keys":"ctrl+shift+a"}),
        ),
        (
            json!({"type":"type","text":"hello"}),
            "type_text",
            json!({"text":"hello"}),
        ),
        (json!({"type":"screenshot"}), "screenshot", json!({})),
    ] {
        mapped(i, "computer", args, native, expected).await;
    }
    assert!(i
        .translate("computer", &json!({"type":"wait"}), (1280, 720))
        .unwrap_err()
        .contains("agentaccess___wait"));
}
#[tokio::test]
async fn google_actions() {
    let i = interface(IDS[4]);
    for (name, native) in [
        ("click", "left_click"),
        ("double_click", "double_click"),
        ("triple_click", "triple_click"),
        ("middle_click", "middle_click"),
        ("right_click", "right_click"),
        ("mouse_down", "left_mouse_down"),
        ("mouse_up", "left_mouse_up"),
        ("move", "move_pointer"),
    ] {
        mapped(
            i,
            name,
            json!({"x":0,"y":999}),
            native,
            json!({"x":0,"y":719}),
        )
        .await;
    }
    for (name, args, native, expected) in [
        ("take_screenshot", json!({}), "screenshot", json!({})),
        (
            "type",
            json!({"text":"hello"}),
            "type_text",
            json!({"text":"hello"}),
        ),
        (
            "press_key",
            json!({"key":"Return"}),
            "key",
            json!({"keys":"Return"}),
        ),
        (
            "hotkey",
            json!({"keys":["Control","a"]}),
            "key",
            json!({"keys":"ctrl+a"}),
        ),
        (
            "scroll",
            json!({"x":999,"y":0,"direction":"right","magnitude":2}),
            "scroll",
            json!({"x":1279,"y":0,"scroll_direction":"Right","scroll_amount":240}),
        ),
        (
            "drag_and_drop",
            json!({"x":0,"y":0,"destination_x":999,"destination_y":999}),
            "left_click_drag",
            json!({"start_x":0,"start_y":0,"end_x":1279,"end_y":719}),
        ),
    ] {
        mapped(i, name, args, native, expected).await;
    }
}
#[test]
fn malformed_arguments() {
    for id in IDS {
        let i = interface(id);
        let name = if id.contains("TOOLSET") {
            "left_click"
        } else if id.starts_with("GOOGLE") {
            "click"
        } else {
            "computer"
        };
        for args in [Value::Null, json!([]), json!({}), json!({"unexpected":1})] {
            assert!(
                i.translate(name, &args, (1280, 720)).is_err(),
                "{id} {args}"
            );
        }
        assert!(i
            .translate("agentaccess___type_text", &json!({"text":"x"}), (1280, 720))
            .is_err());
    }
    let i = interface(IDS[1]);
    for args in [
        json!({"action":"left_click","coordinate":[1]}),
        json!({"action":"left_click","coordinate":[1.5,2]}),
        json!({"action":"scroll","coordinate":[1,2],"scroll_direction":"down","scroll_amount":i64::MAX}),
        json!({"action":"key","text":3}),
        json!({"action":"nope"}),
    ] {
        assert!(i.translate("computer", &args, (1280, 720)).is_err());
    }
    let i = interface(IDS[3]);
    for args in [
        json!({"type":"click","x":1,"y":2,"button":"bad"}),
        json!({"type":"drag","path":[]}),
        json!({"type":"keypress","keys":[1]}),
        json!({"type":"keypress","keys":[]}),
        json!({"type":"scroll","x":1,"y":2,"scroll_x":1,"scroll_y":1}),
    ] {
        assert!(i.translate("computer", &args, (1280, 720)).is_err());
    }
    let i = interface(IDS[4]);
    for x in [-1, 1000] {
        assert!(i
            .translate("click", &json!({"x":x,"y":0}), (1280, 720))
            .is_err());
    }
    assert_eq!(
        i.translate("click", &json!({"x":999,"y":999}), (1920, 1080))
            .unwrap()
            .1,
        json!({"x":1919,"y":1079})
    );
}

#[test]
fn missing_action_fields() {
    let mut cases = vec![];
    for action in [
        "click",
        "double_click",
        "move",
        "scroll",
        "drag",
        "keypress",
        "type",
    ] {
        let args = match action {
            "scroll" => json!({"x":1,"y":2,"scroll_x":0,"scroll_y":120}),
            "drag" => json!({"path":[{"x":1,"y":2},{"x":3,"y":4}]}),
            "keypress" => json!({"keys":["a"]}),
            "type" => json!({"text":"hello"}),
            _ => json!({"x":1,"y":2}),
        };
        cases.push((IDS[3], "computer", args, Some(("type", action))));
    }
    for id in &IDS[..3] {
        for (action, args) in [
            (
                "scroll",
                json!({"coordinate":[1,2],"scroll_direction":"down","scroll_amount":1}),
            ),
            (
                "left_click_drag",
                json!({"coordinate":[1,2],"start_coordinate":[3,4]}),
            ),
            ("hold_key", json!({"text":"a","duration":1})),
            ("type", json!({"text":"hello"})),
            ("key", json!({"text":"a"})),
            ("mouse_move", json!({"coordinate":[1,2]})),
        ] {
            let split = id.contains("TOOLSET");
            cases.push((
                id,
                if split { action } else { "computer" },
                args,
                (!split).then_some(("action", action)),
            ));
        }
    }
    for (name, args) in [
        (
            "scroll",
            json!({"x":1,"y":2,"direction":"down","magnitude":1}),
        ),
        (
            "drag_and_drop",
            json!({"x":1,"y":2,"destination_x":3,"destination_y":4}),
        ),
        ("type", json!({"text":"hello"})),
        ("press_key", json!({"key":"a"})),
        ("hotkey", json!({"keys":["a"]})),
        ("click", json!({"x":1,"y":2})),
    ] {
        cases.push((IDS[4], name, args, None));
    }
    for (id, name, args, discriminator) in cases {
        for field in args.as_object().unwrap().keys() {
            let mut missing = args.clone();
            missing.as_object_mut().unwrap().remove(field);
            if let Some((key, action)) = discriminator {
                missing[key] = json!(action);
            }
            assert_eq!(
                interface(id)
                    .translate(name, &missing, (1280, 720))
                    .unwrap_err(),
                format!("missing {field}"),
                "{id} {name} {missing}"
            );
        }
    }
}

#[test]
fn advertised_enums_are_enforced() {
    for (id, name, args, field) in [
        (IDS[0], "computer", json!({"action":"unknown"}), "action"),
        (IDS[3], "computer", json!({"type":"unknown"}), "type"),
        (
            IDS[3],
            "computer",
            json!({"type":"click","x":1,"y":2,"button":"LEFT"}),
            "button",
        ),
        (
            IDS[0],
            "computer",
            json!({"action":"scroll","coordinate":[1,2],"scroll_direction":"DOWN","scroll_amount":1}),
            "scroll_direction",
        ),
        (
            IDS[2],
            "scroll",
            json!({"coordinate":[1,2],"scroll_direction":"DOWN","scroll_amount":1}),
            "scroll_direction",
        ),
        (
            IDS[4],
            "scroll",
            json!({"x":1,"y":2,"direction":"DOWN","magnitude":1}),
            "direction",
        ),
    ] {
        assert_eq!(
            interface(id)
                .translate(name, &args, (1280, 720))
                .unwrap_err(),
            format!("invalid {field}")
        );
    }
    assert_eq!(
        interface(IDS[3])
            .translate(
                "computer",
                &json!({"type":"scroll","x":1,"y":2,"scroll_x":0,"scroll_y":0}),
                (1280, 720)
            )
            .unwrap_err(),
        "scroll requires a nonzero scroll_x or scroll_y"
    );
}
