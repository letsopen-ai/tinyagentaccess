use super::*;
pub const ACTIONS: &[&str] = &[
    "screenshot",
    "left_click",
    "right_click",
    "middle_click",
    "double_click",
    "triple_click",
    "left_click_drag",
    "mouse_move",
    "left_mouse_down",
    "left_mouse_up",
    "scroll",
    "type",
    "key",
    "hold_key",
];
pub fn schema(action: Option<&str>) -> Value {
    let coordinate = json!({"type":"array","items":{"type":"integer"},"minItems":2,"maxItems":2});
    let mut p = json!({});
    let mut required = vec![];
    let actions = action.map_or_else(|| ACTIONS.to_vec(), |a| vec![a]);
    for a in actions {
        match a {
            "type" | "key" | "hold_key" => {
                p["text"] = json!({"type":"string"});
                if action.is_some() {
                    required.push("text");
                }
            }
            "screenshot" => (),
            _ => {
                p["coordinate"] = coordinate.clone();
                if action.is_some() {
                    required.push("coordinate");
                }
            }
        }
        if matches!(
            a,
            "left_click"
                | "right_click"
                | "middle_click"
                | "double_click"
                | "triple_click"
                | "left_mouse_down"
                | "left_mouse_up"
                | "scroll"
        ) {
            p["text"] = json!({"type":"string"});
        }
        if a == "left_click_drag" {
            p["start_coordinate"] = coordinate.clone();
            if action.is_some() {
                required.push("start_coordinate");
            }
        }
        if a == "scroll" {
            p["scroll_direction"] = json!({"type":"string","enum":["up","down","left","right"]});
            p["scroll_amount"] = json!({"type":"integer"});
            if action.is_some() {
                required.extend(["scroll_direction", "scroll_amount"]);
            }
        }
        if a == "hold_key" {
            p["duration"] = json!({"type":"number"});
            if action.is_some() {
                required.push("duration");
            }
        }
    }
    if action.is_none() {
        p["action"] = json!({"type":"string","enum":ACTIONS});
        required.push("action");
    }
    object(p, &required)
}
pub fn translate(action: &str, a: &Value) -> Result<(String, Value), String> {
    if matches!(action, "wait" | "zoom" | "cursor_position") {
        return Err(rejected());
    }
    if !ACTIONS.contains(&action) {
        return Err("Unknown action".into());
    }
    // Check per-action required fields and reject fields belonging to other actions.
    let mut input = a.clone();
    input.as_object_mut().unwrap().remove("action");
    check(&schema(Some(action)), &input)?;
    let n = match action {
        "mouse_move" => "move_pointer",
        "type" => "type_text",
        _ => action,
    };
    let mut out = json!({});
    match action {
        "screenshot" => (),
        "type" => out["text"] = a["text"].clone(),
        "key" | "hold_key" => {
            out["keys"] = json!(keys(string(a, "text")?));
            if action == "hold_key" {
                out["duration"] = a["duration"].clone();
            }
        }
        _ => {
            let (x, y) = point(a, "coordinate")?;
            if action == "left_click_drag" {
                let (sx, sy) = point(a, "start_coordinate")?;
                out = json!({"start_x":sx,"start_y":sy,"end_x":x,"end_y":y});
            } else {
                out = json!({"x":x,"y":y});
                if let Some(t) = a.get("text") {
                    out["modifiers"] = json!(keys(t.as_str().ok_or("invalid text")?));
                }
                if action == "scroll" {
                    out["scroll_direction"] = json!(direction(string(a, "scroll_direction")?)?);
                    out["scroll_amount"] = json!(notches(integer(a, "scroll_amount")?)?);
                }
            }
        }
    }
    Ok((n.into(), out))
}
