pub mod config;
pub mod dialects;
pub mod replay;
pub mod screenshots;
use base64::Engine;
use config::Config;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use taa_platform::{
    pixels::{Mapping, Point},
    Button, Operation, Platform, ScrollDirection,
};
use tokio::time::Instant;
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
            p["scroll_amount"] = json!({"type":"integer","minimum":-taa_platform::MAX_SCROLL_TICKS,"maximum":taa_platform::MAX_SCROLL_TICKS});
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
    json!({"tools":NAMES.iter().filter(|n|visible(n,c,modern)).map(|n|json!({"name":format!("{PREFIX}{n}"),"description":match *n {"screenshot"=>"Capture desktop; returned image dimensions define the coordinate space for all mouse tools.","execute_sequence"=>"Run at most 20 native actions; each wait 0.001–5 s, cumulative wait ≤ 10 s, cumulative type_text ≤ 10,000 chars, 60 s deadline. screenshot/execute_sequence cannot be nested.","close_session"=>"End session (stateless protocol).",
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
    validate_arguments(n, a, Some(screen))
}
/// Validate argument shape without consulting potentially stale display dimensions.
pub fn validate_shape(n: &str, a: &Value) -> Result<(), String> {
    validate_arguments(n, a, None)
}
fn validate_arguments(n: &str, a: &Value, screen: Option<(u32, u32)>) -> Result<(), String> {
    if !a.is_object() {
        return Err("arguments must be an object".into());
    }
    // PROVISIONAL (not published by AWS): standalone wait validation wording follows sequence sample.
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
            let Some(screen) = screen else {
                continue;
            };
            let bound = if k.ends_with('x') { screen.0 } else { screen.1 };
            if bound == 0 {
                return Err(
                    "native screen size unknown; take a screenshot before using native coordinates"
                        .into(),
                );
            }
            let x = v.as_i64().unwrap();
            if x < 0 || x >= bound as i64 {
                return Err(format!("{k} outside screen bounds"));
            }
        }
    }
    if let Some(t) = a["text"].as_str() {
        if t.chars()
            .any(|c| c.is_control() && !matches!(c, '\r' | '\n' | '\t'))
        {
            return Err(
                "text contains unsupported control characters; only CR, LF and Tab are allowed"
                    .into(),
            );
        }
    }
    if let Some(keys) = a["keys"].as_str() {
        taa_platform::keys::combo(keys, false)?;
    }
    if let Some(keys) = a["modifiers"].as_str() {
        taa_platform::keys::combo(keys, true)?;
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
pub async fn execute(n: &str, a: &Value, c: &Config, p: Arc<dyn Platform>) -> Value {
    if let Err(e) = validate(
        n,
        a,
        if c.agent_access.screen_resolution == "native" {
            (i32::MAX as u32, i32::MAX as u32)
        } else {
            screen(c, p.as_ref())
        },
    ) {
        return client(e);
    }
    // PROVISIONAL (not published by AWS): standalone wait success text.
    if n == "wait" {
        tokio::time::sleep(Duration::from_secs_f64(a["duration"].as_f64().unwrap())).await;
        return result("Wait completed", false);
    }
    let n = n.to_owned();
    let a = a.clone();
    let c = c.clone();
    match tokio::task::spawn_blocking(move || execute_blocking(&n, &a, &c, p.as_ref())).await {
        Ok(v) => v,
        Err(e) => result(format!("SERVER_ERROR: desktop worker failed: {e}"), true),
    }
}
fn is_pointer(n: &str) -> bool {
    matches!(
        n,
        "move_pointer"
            | "left_click"
            | "double_click"
            | "triple_click"
            | "right_click"
            | "middle_click"
            | "left_click_drag"
            | "left_mouse_down"
            | "left_mouse_up"
            | "scroll"
    )
}
fn execute_blocking(n: &str, a: &Value, c: &Config, p: &dyn Platform) -> Value {
    let native = c.agent_access.screen_resolution == "native";
    if n == "screenshot" {
        let outcome = p
            .capture()
            .and_then(|f| {
                taa_platform::pixels::render(
                    f,
                    native,
                    a["include_cursor"].as_bool().unwrap_or(false),
                )
            })
            .and_then(|f| {
                taa_platform::pixels::encode(&f, c.agent_access.screen_image_format == "jpeg")
            });
        return match outcome {
            Ok((bytes, mime)) => {
                json!({"content":[{"type":"image","data":base64::engine::general_purpose::STANDARD.encode(bytes),"mimeType":mime}],"isError":false})
            }
            Err(e) => result(format!("SERVER_ERROR: {e}"), true),
        };
    }
    // Only pointer actions need geometry; keyboard input must not initiate Screenshot consent.
    let geometry = if is_pointer(n) {
        p.capture_geometry()
    } else {
        Ok(p.screen_size())
    };
    let f = match geometry {
        Ok(s) => s,
        Err(e) => return result(format!("SERVER_ERROR: {e}"), true),
    };
    if let Err(e) = validate(n, a, if native { f } else { (1280, 720) }) {
        return client(e);
    }
    let m = Mapping::new(f, native);
    let point = |x: &str, y: &str| {
        m.inverse(Point {
            x: a[x].as_i64().unwrap() as i32,
            y: a[y].as_i64().unwrap() as i32,
        })
    };
    let mods = taa_platform::keys::combo(a["modifiers"].as_str().unwrap_or(""), true).unwrap();
    let op = match n {
        "move_pointer" => Operation::Move(point("x", "y")),
        "left_mouse_down" | "left_mouse_up" => {
            Operation::Button(point("x", "y"), n == "left_mouse_down", mods)
        }
        "left_click_drag" => Operation::Drag(point("start_x", "start_y"), point("end_x", "end_y")),
        "scroll" => Operation::Scroll(
            point("x", "y"),
            match a["scroll_direction"].as_str().unwrap() {
                "Up" => ScrollDirection::Up,
                "Down" => ScrollDirection::Down,
                "Left" => ScrollDirection::Left,
                _ => ScrollDirection::Right,
            },
            a["scroll_amount"].as_i64().unwrap(),
            mods,
        ),
        "key" | "hold_key" => Operation::Key(
            taa_platform::keys::combo(a["keys"].as_str().unwrap(), false).unwrap(),
            Duration::from_secs_f64(a["duration"].as_f64().unwrap_or(0.)),
        ),
        "type_text" => Operation::Text(
            a["text"]
                .as_str()
                .unwrap()
                .replace("\r\n", "\n")
                .replace('\r', "\n"),
        ),
        "left_click" | "double_click" | "triple_click" | "right_click" | "middle_click" => {
            Operation::Click(
                point("x", "y"),
                match n {
                    "right_click" => Button::Right,
                    "middle_click" => Button::Middle,
                    _ => Button::Left,
                },
                match n {
                    "double_click" => 2,
                    "triple_click" => 3,
                    _ => 1,
                },
                mods,
            )
        }
        _ => return client("Unknown tool"),
    };
    match p.perform(op) {
        Ok(()) => result("Action completed", false),
        Err(e) => result(format!("SERVER_ERROR: {e}"), true),
    }
}

// PROVISIONAL (not published by AWS): short names accepted from SAMPLE S-agentsmd only.
fn short(a: &Value) -> &str {
    a["type"]
        .as_str()
        .unwrap_or("")
        .strip_prefix(PREFIX)
        .unwrap_or(a["type"].as_str().unwrap_or(""))
}
// PROVISIONAL (not published by AWS): stabilization length and injectable local timing.
pub const STABILIZATION: Duration = Duration::from_millis(300);
#[derive(Clone, Copy)]
pub struct SequenceLimits {
    pub deadline: Duration,
    pub stabilization: Duration,
}
impl Default for SequenceLimits {
    fn default() -> Self {
        Self {
            deadline: Duration::from_secs(60),
            stabilization: STABILIZATION,
        }
    }
}
pub async fn sequence(a: &Value, c: &Config, p: Arc<dyn Platform>) -> Value {
    sequence_with_limits(a, c, p, SequenceLimits::default()).await
}
pub async fn sequence_with_limits(
    a: &Value,
    c: &Config,
    p: Arc<dyn Platform>,
    limits: SequenceLimits,
) -> Value {
    sequence_recorded(a, c, p, limits, None).await
}
pub async fn sequence_recorded(
    a: &Value,
    c: &Config,
    p: Arc<dyn Platform>,
    limits: SequenceLimits,
    replay: Option<&replay::Context>,
) -> Value {
    let empty = vec![];
    let actions = a["actions"].as_array().unwrap_or(&empty);
    let mut reason = None;
    let mut at = 0;
    // Allow only floating-point summation noise at the inclusive 10-second boundary.
    const WAIT_LIMIT: f64 = 10.0 + 1e-9;
    let mut wait = 0.0;
    let mut chars = 0;
    // PROVISIONAL (not published by AWS): count first, then ordered argument checks,
    // then cumulative totals; empty/malformed arrays and session/forwarded tools rejected.
    if !a["actions"].is_array() {
        reason = Some("actions must be an array".into());
    } else if actions.len() > 20 {
        at = 20;
        reason = Some(format!("too many actions: {} > 20", actions.len()));
    } else if actions.is_empty() {
        reason = Some("actions must not be empty".into());
    }
    let native = c.agent_access.screen_resolution == "native";
    let size = screen(c, p.as_ref());
    if reason.is_none() {
        if let Err(e) = validate_shape("execute_sequence", a) {
            reason = Some(e);
        }
    }
    let mut wait_crossed = 0;
    let mut chars_crossed = 0;
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
        let e = if matches!(n, "screenshot" | "execute_sequence") {
            Err(format!("{n} cannot be nested in execute_sequence"))
        } else if !NAMES.contains(&n)
            || matches!(n, "close_session" | "get_session_info")
            || !visible(n, c, false)
        {
            Err("Unknown tool".into())
        } else if native {
            validate_shape(n, &args)
        } else {
            validate(n, &args, size)
        };
        if let Err(e) = e {
            reason = Some(format!("action {i}: {e}"));
            break;
        }
        if n == "wait" {
            let previous = wait;
            wait += action["duration"].as_f64().unwrap();
            if previous <= WAIT_LIMIT && wait > WAIT_LIMIT {
                wait_crossed = i;
            }
        }
        if n == "type_text" {
            let previous = chars;
            // Unicode scalar values, matching standalone validation (not UTF-8 bytes).
            chars += action["text"].as_str().unwrap().chars().count();
            if previous <= 10000 && chars > 10000 {
                chars_crossed = i;
            }
        }
    }
    // The displayed wait total rounds to one decimal, independently of comparison precision.
    if reason.is_none() && wait > WAIT_LIMIT {
        at = wait_crossed;
        reason = Some(format!("cumulative wait {wait:.1}s exceeds 10.0s"));
    }
    if reason.is_none() && chars > 10000 {
        at = chars_crossed;
        reason = Some(format!("cumulative type_text length {chars} exceeds 10000"));
    }
    // Native bounds depend on fresh geometry, but malformed sequences must never
    // initiate capture (which may prompt for portal consent).
    if reason.is_none() && native && actions.iter().any(|action| is_pointer(short(action))) {
        let platform = p.clone();
        let size = match tokio::task::spawn_blocking(move || platform.capture_geometry()).await {
            Ok(Ok(size)) => size,
            Ok(Err(e)) => return result(format!("SERVER_ERROR: {e}"), true),
            Err(e) => return result(format!("SERVER_ERROR: desktop worker failed: {e}"), true),
        };
        for (i, action) in actions.iter().enumerate() {
            let n = short(action);
            if is_pointer(n) {
                let mut args = action.clone();
                args.as_object_mut().unwrap().remove("type");
                if let Err(e) = validate(n, &args, size) {
                    at = i;
                    reason = Some(format!("action {i}: {e}"));
                    break;
                }
            }
        }
    }
    if let Some(reason) = reason {
        return result(json!({"sequence_summary":{"submitted":actions.len(),"executed":0,"succeeded":0,"stopped_at_index":at,"stop_reason":format!("CLIENT_ERROR: Rejected before dispatch: {reason}"),"guidance":"Correct the sequence and retry. No desktop action ran, so no screenshot is needed."},"action_results":actions.iter().enumerate().map(|(i,a)|json!({"index":i,"action":short(a),"status":"skipped","reason":"sequence rejected before dispatch"})).collect::<Vec<_>>()}).to_string(), true);
    }
    let deadline = Instant::now() + limits.deadline;
    let mut rows = vec![];
    let mut stop: Option<String> = None;
    let mut stopped_at = None;
    let mut executed = 0;
    let mut succeeded = 0;
    // PROVISIONAL (not published by AWS): error/skipped statuses and failure guidance.
    for (i, a) in actions.iter().enumerate() {
        let n = short(a);
        if stop.is_none() && Instant::now() >= deadline {
            stop = Some(format!("Deadline reached before action {i}"));
            stopped_at = Some(i);
        }
        if stop.is_some() {
            rows.push(json!({"index":i,"action":n,"status":"skipped","reason":"sequence stopped before action"}));
            continue;
        }
        let mut args = a.clone();
        args.as_object_mut().unwrap().remove("type");
        let event = if n == "wait" {
            if let Some(context) = replay {
                context.begin(n, &args, c, p.clone()).await
            } else {
                None
            }
        } else {
            None
        };
        let t = Instant::now();
        let r = if n == "wait" {
            let end = t + Duration::from_secs_f64(a["duration"].as_f64().unwrap());
            // Clamp in-progress waits; native blocking calls cannot safely be interrupted.
            tokio::time::sleep_until(end.min(deadline)).await;
            if Instant::now() >= deadline {
                // PROVISIONAL: i + 1 is the first undispatched action; stopped_at_index
                // remains i, the interrupted wait (AWS documents only the message text).
                result(format!("Deadline reached before action {}", i + 1), true)
            } else {
                result("Wait completed", false)
            }
        } else {
            replay::execute(n, &args, c, p.clone(), replay).await
        };
        replay::finish(event, &r).await;
        executed += 1;
        if r["isError"] == true {
            let reason = r["content"][0]["text"].as_str().unwrap_or("Action failed");
            stop = Some(reason.into());
            stopped_at = Some(i);
            rows.push(json!({"index":i,"action":n,"status":"error","duration_ms":t.elapsed().as_millis(),"reason":reason}));
        } else {
            succeeded += 1;
            rows.push(json!({"index":i,"action":n,"status":"success","duration_ms":t.elapsed().as_millis()}));
        }
    }
    let mut contents = vec![];
    let mut terminal = None;
    if stop.is_none() {
        if actions.last().is_some_and(|a| short(a) != "wait") {
            tokio::time::sleep_until((Instant::now() + limits.stabilization).min(deadline)).await;
        }
        if Instant::now() >= deadline {
            stop = Some("Deadline reached before terminal screenshot".into());
        } else if !c.agent_access.computer_vision {
            // PROVISIONAL (not published by AWS): vision-disabled terminal is skipped successfully.
            terminal = Some(json!({"status":"skipped","reason":"computer vision disabled"}));
        } else {
            let t = Instant::now();
            let r = execute("screenshot", &json!({}), c, p).await;
            // PROVISIONAL (not published by AWS): terminal capture failure is an error, without image.
            if r["isError"] == true || Instant::now() >= deadline {
                let reason = if Instant::now() >= deadline {
                    "Deadline reached during terminal screenshot"
                } else {
                    r["content"][0]["text"]
                        .as_str()
                        .unwrap_or("Screenshot failed")
                };
                stop = Some(reason.into());
                terminal = Some(
                    json!({"status":"error","duration_ms":t.elapsed().as_millis(),"reason":reason}),
                );
            } else {
                terminal = Some(json!({"status":"success","duration_ms":t.elapsed().as_millis()}));
                contents.extend(r["content"].as_array().unwrap().clone());
            }
        }
    }
    let mut summary = json!({"submitted":actions.len(),"executed":executed,"succeeded":succeeded,"guidance":if stop.is_some(){"Take a screenshot before retrying."}else{""}});
    if let Some(reason) = &stop {
        summary["stop_reason"] = json!(reason);
        summary["stopped_at_index"] = json!(stopped_at);
    }
    let mut body = json!({"sequence_summary":summary,"action_results":rows});
    if let Some(terminal) = terminal {
        body["terminal_screenshot"] = terminal;
    }
    contents.insert(0, text(body.to_string()));
    json!({"content":contents,"isError":stop.is_some()})
}

#[cfg(test)]
mod sequence_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct CountingPlatform {
        geometry_calls: AtomicUsize,
        performed: AtomicUsize,
    }

    impl Platform for CountingPlatform {
        fn name(&self) -> &str {
            "counting"
        }
        fn screen_size(&self) -> (u32, u32) {
            (1920, 1080)
        }
        fn capture_geometry(&self) -> Result<(u32, u32), String> {
            self.geometry_calls.fetch_add(1, Ordering::SeqCst);
            Ok((100, 80))
        }
        fn capture(&self) -> Result<taa_platform::Frame, String> {
            Ok(taa_platform::Frame::solid(100, 80, [0, 0, 0, 255]))
        }
        fn perform(&self, _: Operation) -> Result<(), String> {
            self.performed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn native_config() -> Config {
        let mut config = Config::default();
        config.agent_access.screen_resolution = "native".into();
        config
    }

    async fn assert_rejected(actions: Value, index: usize, captures: usize) {
        let platform = Arc::new(CountingPlatform::default());
        let response = sequence(
            &json!({"actions": actions}),
            &native_config(),
            platform.clone(),
        )
        .await;
        assert_eq!(response["isError"], true);
        let body: Value =
            serde_json::from_str(response["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(body["sequence_summary"]["stop_reason"]
            .as_str()
            .unwrap()
            .starts_with("CLIENT_ERROR: Rejected before dispatch:"));
        assert_eq!(body["sequence_summary"]["stopped_at_index"], index);
        assert_eq!(body["sequence_summary"]["executed"], 0);
        assert_eq!(platform.geometry_calls.load(Ordering::SeqCst), captures);
        assert_eq!(platform.performed.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn malformed_pointer_does_not_capture_geometry() {
        assert_rejected(json!([{"type":"left_click","x":"invalid","y":0}]), 0, 0).await;
        assert_rejected(
            json!([{"type":"left_click","x":0,"y":0}, {"type":"key","keys":42}]),
            1,
            0,
        )
        .await;
    }

    #[tokio::test]
    async fn invalid_actions_and_limits_do_not_capture_geometry() {
        for action in [
            json!({"type":"unknown"}),
            json!({"type":"execute_sequence","actions":[]}),
            json!({"type":"screenshot"}),
            json!(null),
        ] {
            assert_rejected(json!([{"type":"left_click","x":0,"y":0}, action]), 1, 0).await;
        }
        assert_rejected(json!([{"type":"left_click","x":0,"y":0}, {"type":"wait","duration":5}, {"type":"wait","duration":5}, {"type":"wait","duration":1}, {"type":"key","keys":"a"}]), 3, 0).await;
        assert_rejected(json!([{"type":"left_click","x":0,"y":0}, {"type":"type_text","text":"a".repeat(6000)}, {"type":"type_text","text":"b".repeat(5000)}, {"type":"key","keys":"a"}]), 2, 0).await;
        assert_rejected(
            json!(vec![json!({"type":"left_click","x":0,"y":0}); 21]),
            20,
            0,
        )
        .await;
        let mut config = native_config();
        config.agent_access.computer_input = false;
        let platform = Arc::new(CountingPlatform::default());
        let response = sequence(
            &json!({"actions":[{"type":"left_click","x":0,"y":0}]}),
            &config,
            platform.clone(),
        )
        .await;
        assert_eq!(response["isError"], true);
        assert_eq!(platform.geometry_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn bounds_use_refreshed_geometry() {
        assert_rejected(
            json!([{"type":"key","keys":"a"}, {"type":"left_click","x":100,"y":0}]),
            1,
            1,
        )
        .await;
    }

    #[tokio::test]
    async fn valid_sequence_executes() {
        let platform = Arc::new(CountingPlatform::default());
        let response = sequence(&json!({"actions":[{"type":"left_click","x":99,"y":79}, {"type":"type_text","text":"hello"}]}), &native_config(), platform.clone()).await;
        assert_eq!(response["isError"], false);
        assert_eq!(platform.performed.load(Ordering::SeqCst), 2);
        // Preflight refresh and the pointer action's existing dispatch refresh.
        assert_eq!(platform.geometry_calls.load(Ordering::SeqCst), 2);
    }
}
