pub mod config;
use base64::Engine;
use config::Config;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use taa_platform::Platform;
pub const PREFIX: &str = "agentaccess___";
pub const NAMES: &[&str] = &[
    "left_click",
    "double_click",
    "triple_click",
    "right_click",
    "middle_click",
    "left_click_drag",
    "left_mouse_down",
    "left_mouse_up",
    "move_pointer",
    "scroll",
    "type_text",
    "key",
    "hold_key",
    "screenshot",
    "execute_sequence",
    "wait",
    "get_session_info",
    "close_session",
];
pub fn text(s: impl Into<String>) -> Value {
    json!({"type":"text","text":s.into()})
}
pub fn result(s: impl Into<String>, error: bool) -> Value {
    json!({"content":[text(s)],"isError":error})
}
pub fn client(s: impl Into<String>) -> Value {
    result(format!("CLIENT_ERROR: {}", s.into()), true)
}
pub fn visible(n: &str, c: &Config, modern: bool) -> bool {
    match n {
        "close_session" => modern,
        "screenshot" => c.agent_access.computer_vision,
        "wait" | "get_session_info" => true,
        _ => c.agent_access.computer_input,
    }
}
// [TINY-PROPOSAL] not AWS-verified, see docs/aws-parity.md §5.2
// All schemas are generated here; limits marked below are documented facts.
pub fn schema(n: &str, modern: bool) -> Value {
    let mut p = json!({});
    let mut required = Vec::<String>::new();
    let fields: Vec<&str> = match n {
        "left_click_drag" => vec!["start_x", "start_y", "end_x", "end_y"],
        "move_pointer" | "left_click" | "double_click" | "triple_click" | "right_click"
        | "middle_click" | "left_mouse_down" | "left_mouse_up" | "scroll" => vec!["x", "y"],
        _ => vec![],
    };
    for f in fields {
        p[f] = json!({"type":"integer"});
        required.push(f.into());
    }
    if matches!(
        n,
        "left_click"
            | "double_click"
            | "triple_click"
            | "right_click"
            | "middle_click"
            | "left_mouse_down"
            | "left_mouse_up"
            | "scroll"
    ) {
        p["modifiers"] = json!({"type":"string"});
    }
    match n {
        "scroll" => {
            p["scroll_direction"] = json!({"type":"string","enum":["Up","Down","Left","Right"]});
            p["scroll_amount"] = json!({"type":"integer"});
            required.extend(["scroll_direction".into(), "scroll_amount".into()]);
        } // [DOC] ticks, 120 per notch
        "wait" | "hold_key" => {
            let (min, max) = if n == "wait" {
                (0.001, 5.0)
            } else {
                (1.0, 30.0)
            };
            p["duration"] = json!({"type":"number","minimum":min,"maximum":max});
            required.push("duration".into());
        } // [DOC] duration limits
        "type_text" => {
            p["text"] = json!({"type":"string","maxLength":10000});
            required.push("text".into());
        } // [DOC] Unicode character limit
        "screenshot" => p["include_cursor"] = json!({"type":"boolean","default":false}),
        "close_session" => {
            p["expire_streaming_session"] = json!({"type":"boolean","default":false})
        }
        "execute_sequence" => {
            p["actions"] = json!({"type":"array","maxItems":20,"items":{"type":"object","properties":{"type":{"type":"string"}},"required":["type"],"additionalProperties":true}});
            required.push("actions".into());
        } // [SAMPLE] maxItems
        _ => (),
    }
    if matches!(n, "key" | "hold_key") {
        p["keys"] = json!({"type":"string"});
        required.push("keys".into());
    }
    if modern {
        p["mcpSessionId"] = json!({"type":"string"});
        required.push("mcpSessionId".into());
    }
    json!({"type":"object","properties":p,"required":required,"additionalProperties":false})
}
pub fn list(c: &Config, modern: bool) -> Value {
    json!({"tools":NAMES.iter().filter(|n|visible(n,c,modern)).map(|n|json!({"name":format!("{PREFIX}{n}"),"description":match *n {"screenshot"=>"Capture desktop; returned image dimensions define the coordinate space for all mouse tools.","execute_sequence"=>"Run at most 20 actions; cumulative wait ≤ 10 s, cumulative type_text ≤ 10,000 chars, 60 s deadline. screenshot/execute_sequence cannot be nested.","close_session"=>"End session (stateless protocol).",
"left_click"=>"Left click at coordinates, with optional modifiers.",
"double_click"=>"Double left click at coordinates.",
"triple_click"=>"Triple left click at coordinates.",
"right_click"=>"Right click at coordinates.",
"middle_click"=>"Middle click at coordinates.",
"left_click_drag"=>"Drag the left mouse button from start to end coordinates.",
"left_mouse_down"=>"Press and hold the left mouse button at coordinates.",
"left_mouse_up"=>"Release the left mouse button at coordinates.",
"move_pointer"=>"Move the mouse pointer to coordinates.",
"scroll"=>"Scroll Up, Down, Left, or Right at coordinates; 120 ticks equal one notch.",
"type_text"=>"Type up to 10,000 characters using keyboard events.",
"key"=>"Press a key or a + joined combination, such as ctrl+c.",
"hold_key"=>"Hold a key or combination for 1 to 30 seconds.",
"wait"=>"Pause for 0.001 to 5 seconds without desktop interaction.",
"get_session_info"=>"Return local desktop session metadata.",_=>n},"inputSchema":schema(n,modern)})).collect::<Vec<_>>()})
}
pub fn validate(n: &str, a: &Value, screen: (u32, u32)) -> Result<(), String> {
    if !a.is_object() {
        return Err("arguments must be an object".into());
    }
    if n == "wait"
        && !a["duration"]
            .as_f64()
            .is_some_and(|v| (0.001..=5.0).contains(&v))
    {
        return Err("wait duration must be a number between 0.001 and 5.0 seconds".into());
    }
    let s = schema(n, false);
    for r in s["required"].as_array().unwrap() {
        let k = r.as_str().unwrap();
        if a.get(k).is_none() {
            return Err(format!("missing {k}"));
        }
    }
    for (k, v) in a.as_object().unwrap() {
        if k == "mcpSessionId" {
            continue;
        }
        let Some(rule) = s["properties"].get(k) else {
            return Err(format!("unknown argument {k}"));
        };
        let valid = match rule["type"].as_str().unwrap() {
            "integer" => v.as_i64().is_some(),
            "number" => v.as_f64().is_some(),
            "string" => v.is_string(),
            "boolean" => v.is_boolean(),
            "array" => v.is_array(),
            _ => true,
        };
        if !valid {
            return Err(format!("invalid {k}"));
        }
        if let Some(e) = rule.get("enum") {
            if !e.as_array().unwrap().contains(v) {
                return Err(format!("invalid {k}"));
            }
        }
        if let Some(max) = rule["maxLength"].as_u64() {
            if v.as_str().unwrap().chars().count() > max as usize {
                return Err(format!("{k} length exceeds {max}"));
            }
        }
        if let Some(min) = rule["minimum"].as_f64() {
            let x = v.as_f64().unwrap();
            if x < min || x > rule["maximum"].as_f64().unwrap() {
                return Err(format!("invalid {k}"));
            }
        }
        if matches!(
            k.as_str(),
            "x" | "start_x" | "end_x" | "y" | "start_y" | "end_y"
        ) {
            let bound = if k.ends_with('x') { screen.0 } else { screen.1 };
            let x = v.as_i64().unwrap();
            if x < 0 || x >= bound as i64 {
                return Err(format!("{k} outside screen bounds"));
            }
        }
    }
    Ok(())
}
pub fn screen(c: &Config, p: &dyn Platform) -> (u32, u32) {
    if c.agent_access.screen_resolution == "native" {
        p.screen_size()
    } else {
        (1280, 720)
    }
}
pub async fn execute(n: &str, a: &Value, c: &Config, p: &dyn Platform) -> Value {
    if let Err(e) = validate(n, a, screen(c, p)) {
        return client(e);
    }
    if n == "wait" {
        tokio::time::sleep(Duration::from_secs_f64(a["duration"].as_f64().unwrap())).await;
        return result("Wait completed", false);
    }
    let outcome = match n {
        "screenshot" => {
            return match p.screenshot(a["include_cursor"].as_bool().unwrap_or(false)) {
                Ok(s) => {
                    json!({"content":[{"type":"image","data":base64::engine::general_purpose::STANDARD.encode(s.bytes),"mimeType":s.mime_type}],"isError":false})
                }
                Err(e) => result(format!("SERVER_ERROR: {e}"), true),
            }
        }
        "move_pointer" => p.mouse_move(a),
        "left_mouse_down" => p.mouse_down(a),
        "left_mouse_up" => p.mouse_up(a),
        "left_click_drag" => p.drag(a),
        "scroll" => p.scroll(a),
        "key" => p.key(a["keys"].as_str().unwrap()),
        "hold_key" => p.hold_key(a["keys"].as_str().unwrap(), a["duration"].as_f64().unwrap()),
        "type_text" => p.type_text(a["text"].as_str().unwrap()),
        "left_click" | "double_click" | "triple_click" | "right_click" | "middle_click" => p.click(
            match n {
                "right_click" => "right",
                "middle_click" => "middle",
                _ => "left",
            },
            match n {
                "double_click" => 2,
                "triple_click" => 3,
                _ => 1,
            },
            a,
        ),
        _ => return client("Unknown tool"),
    };
    match outcome {
        Ok(()) => result("Action completed", false),
        Err(e) => result(format!("SERVER_ERROR: {e}"), true),
    }
}
fn short(a: &Value) -> &str {
    a["type"]
        .as_str()
        .unwrap_or("")
        .strip_prefix(PREFIX)
        .unwrap_or(a["type"].as_str().unwrap_or(""))
}
pub async fn sequence(a: &Value, c: &Config, p: &dyn Platform) -> Value {
    let Some(actions) = a["actions"].as_array() else {
        return client("actions must be an array");
    };
    let mut reason = None;
    let mut at = 0;
    let mut wait = 0.0;
    let mut chars = 0;
    if actions.len() > 20 {
        reason = Some(format!("too many actions: {} > 20", actions.len()));
    }
    for (i, action) in actions.iter().enumerate() {
        if reason.is_some() {
            break;
        }
        at = i;
        let n = short(action);
        let mut args = action.clone();
        if let Some(o) = args.as_object_mut() {
            o.remove("type");
        }
        let e = if !NAMES.contains(&n)
            || matches!(
                n,
                "screenshot" | "execute_sequence" | "close_session" | "get_session_info"
            )
            || !visible(n, c, false)
        {
            Err("action cannot be nested or is unavailable".into())
        } else {
            validate(n, &args, screen(c, p))
        };
        if let Err(e) = e {
            reason = Some(format!("action {i}: {e}"));
            break;
        }
        if n == "wait" {
            wait += action["duration"].as_f64().unwrap();
        }
        if n == "type_text" {
            chars += action["text"].as_str().unwrap().chars().count();
        }
    }
    if reason.is_none() && wait > 10.0 {
        reason = Some(format!("cumulative wait {wait:.1}s exceeds 10.0s"));
    }
    if reason.is_none() && chars > 10000 {
        reason = Some(format!("cumulative type_text length {chars} exceeds 10000"));
    }
    if let Some(reason) = reason {
        let body = json!({"sequence_summary":{"submitted":actions.len(),"executed":0,"succeeded":0,"stopped_at_index":at,"stop_reason":reason,"guidance":"Correct the sequence and retry. No desktop action ran, so no screenshot is needed."},"action_results":actions.iter().enumerate().map(|(i,a)|json!({"index":i,"action":short(a),"status":"skipped","reason":"sequence rejected before dispatch"})).collect::<Vec<_>>()});
        let mut r = result(body.to_string(), true);
        r["content"].as_array_mut().unwrap().push(text(format!(
            "CLIENT_ERROR: Rejected before dispatch: {reason}"
        )));
        return r;
    }
    let start = Instant::now();
    let mut rows = vec![];
    let mut failed = false;
    let mut executed = 0;
    let mut succeeded = 0;
    for (i, a) in actions.iter().enumerate() {
        let n = short(a);
        if failed || start.elapsed() >= Duration::from_secs(60) {
            failed = true;
            rows.push(json!({"index":i,"action":n,"status":"skipped","reason":"Deadline reached or previous action failed"}));
            continue;
        }
        let mut args = a.clone();
        args.as_object_mut().unwrap().remove("type");
        let t = Instant::now();
        let r = execute(n, &args, c, p).await;
        executed += 1;
        failed = r["isError"] == true;
        if !failed {
            succeeded += 1;
        }
        rows.push(json!({"index":i,"action":n,"status":if failed{"failed"}else{"success"},"duration_ms":t.elapsed().as_millis(),"result":r}));
    }
    let mut contents = vec![];
    let mut terminal = json!({"status":"skipped"});
    if !failed {
        let r = execute("screenshot", &json!({}), c, p).await;
        failed = r["isError"] == true;
        terminal = json!({"status":if failed{"failed"}else{"success"}});
        if !failed {
            contents.extend(r["content"].as_array().unwrap().clone());
        }
    }
    contents.insert(0,text(json!({"sequence_summary":{"submitted":actions.len(),"executed":executed,"succeeded":succeeded,"guidance":if failed{"Take a screenshot before retrying."}else{""}},"action_results":rows,"terminal_screenshot":terminal}).to_string()));
    json!({"content":contents,"isError":failed})
}
