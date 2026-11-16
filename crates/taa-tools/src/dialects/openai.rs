use super::*;
pub fn schema() -> Value {
    object(
        json!({"type":{"type":"string","enum":["click","double_click","drag","move","scroll","keypress","type","screenshot"]},"x":{"type":"integer"},"y":{"type":"integer"},"button":{"type":"string","enum":["left","right","middle"]},"path":{"type":"array","minItems":2,"items":{"type":"object","properties":{"x":{"type":"integer"},"y":{"type":"integer"}},"required":["x","y"],"additionalProperties":false}},"scroll_x":{"type":"integer"},"scroll_y":{"type":"integer"},"keys":{"type":"array","items":{"type":"string"}},"text":{"type":"string"}}),
        &["type"],
    )
}
pub fn translate(a: &Value) -> Result<(String, Value), String> {
    let action = string(a, "type")?;
    let mut out = json!({});
    let n = match action {
        "wait" => return Err(rejected()),
        "screenshot" => "screenshot",
        "type" => {
            out["text"] = json!(string(a, "text")?);
            "type_text"
        }
        "keypress" => {
            out["keys"] = json!(joined(a, "keys")?);
            "key"
        }
        "drag" => {
            let path = a["path"]
                .as_array()
                .filter(|p| p.len() >= 2)
                .ok_or("path must contain at least two points")?;
            for p in path {
                check(
                    &object(
                        json!({"x":{"type":"integer"},"y":{"type":"integer"}}),
                        &["x", "y"],
                    ),
                    p,
                )?;
            }
            let first = &path[0];
            let last = path.last().unwrap();
            out = json!({"start_x":first["x"],"start_y":first["y"],"end_x":last["x"],"end_y":last["y"]});
            "left_click_drag"
        }
        "move" | "click" | "double_click" | "scroll" => {
            out = json!({"x":integer(a,"x")?,"y":integer(a,"y")?});
            match action {
                "move" => "move_pointer",
                "double_click" => "double_click",
                "click" => match a.get("button").map(|v| v.as_str().unwrap_or("")) {
                    None | Some("left") => "left_click",
                    Some("right") => "right_click",
                    Some("middle") => "middle_click",
                    _ => return Err("invalid button".into()),
                },
                _ => {
                    let (d, t) = scroll(integer(a, "scroll_x")?, integer(a, "scroll_y")?)?;
                    out["scroll_direction"] = json!(d);
                    out["scroll_amount"] = json!(t);
                    "scroll"
                }
            }
        }
        _ => return Err("Unknown action".into()),
    };
    Ok((n.into(), out))
}
// PROVISIONAL: pixel-like vendor deltas treated as native ticks, one axis per call.
fn scroll(x: i64, y: i64) -> Result<(&'static str, i64), String> {
    if x != 0 && y != 0 {
        return Err("diagonal scroll is unsupported; send separate axis calls".into());
    }
    let (d, n) = if x != 0 {
        (if x < 0 { "Left" } else { "Right" }, x)
    } else {
        (if y < 0 { "Up" } else { "Down" }, y)
    };
    Ok((d, n.checked_abs().ok_or("scroll amount overflow")?))
}
#[test]
fn scroll_units() {
    assert_eq!(scroll(-120, 0).unwrap(), ("Left", 120));
    assert_eq!(scroll(0, -120).unwrap(), ("Up", 120));
    assert!(scroll(1, 1).is_err());
    assert!(scroll(i64::MIN, 0).is_err());
}
