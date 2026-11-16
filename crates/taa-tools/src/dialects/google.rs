use super::*;
pub const ACTIONS: &[&str] = &[
    "take_screenshot",
    "click",
    "double_click",
    "triple_click",
    "middle_click",
    "right_click",
    "mouse_down",
    "mouse_up",
    "move",
    "type",
    "press_key",
    "hotkey",
    "scroll",
    "drag_and_drop",
];
pub fn schema(name: &str) -> Value {
    let mut p = json!({});
    let mut r = vec![];
    match name {
        "take_screenshot" => (),
        "type" => {
            p["text"] = json!({"type":"string"});
            r.push("text");
        }
        "press_key" => {
            p["key"] = json!({"type":"string"});
            r.push("key");
        }
        "hotkey" => {
            p["keys"] = json!({"type":"array","items":{"type":"string"}});
            r.push("keys");
        }
        _ => {
            for f in if name == "drag_and_drop" {
                vec!["x", "y", "destination_x", "destination_y"]
            } else {
                vec!["x", "y"]
            } {
                p[f] = json!({"type":"integer","minimum":0,"maximum":999});
                r.push(f);
            }
            if name == "scroll" {
                p["direction"] = json!({"type":"string","enum":["up","down","left","right"]});
                p["magnitude"] = json!({"type":"integer"});
                r.extend(["direction", "magnitude"]);
            }
        }
    }
    object(p, &r)
}
// PROVISIONAL: nearest pixel with inclusive endpoints, integer arithmetic.
fn grid(n: i64, extent: u32) -> Result<i64, String> {
    if !(0..=999).contains(&n) || extent == 0 {
        return Err("invalid normalized coordinate or unknown screen size".into());
    }
    Ok((n * i64::from(extent - 1) + 499) / 999)
}
pub fn translate(name: &str, a: &Value, size: (u32, u32)) -> Result<(String, Value), String> {
    let mut out = json!({});
    let n = match name {
        "take_screenshot" => "screenshot",
        "type" => {
            out["text"] = a["text"].clone();
            "type_text"
        }
        "press_key" => {
            out["keys"] = json!(keys(string(a, "key")?));
            "key"
        }
        "hotkey" => {
            out["keys"] = json!(joined(a, "keys")?);
            "key"
        }
        _ => {
            let x = grid(integer(a, "x")?, size.0)?;
            let y = grid(integer(a, "y")?, size.1)?;
            out = json!({"x":x,"y":y});
            match name {
                "click" => "left_click",
                "mouse_down" => "left_mouse_down",
                "mouse_up" => "left_mouse_up",
                "move" => "move_pointer",
                "drag_and_drop" => {
                    out = json!({"start_x":x,"start_y":y,"end_x":grid(integer(a,"destination_x")?,size.0)?,"end_y":grid(integer(a,"destination_y")?,size.1)?});
                    "left_click_drag"
                }
                "scroll" => {
                    out["scroll_direction"] = json!(direction(string(a, "direction")?)?);
                    out["scroll_amount"] = json!(notches(integer(a, "magnitude")?)?);
                    "scroll"
                }
                _ => name,
            }
        }
    };
    Ok((n.into(), out))
}
#[test]
fn grid_edges_and_rounding() {
    for extent in [1, 720, 1280, 1920, u32::MAX] {
        assert_eq!(grid(0, extent).unwrap(), 0);
        assert_eq!(grid(999, extent).unwrap(), i64::from(extent - 1));
    }
    assert_eq!(grid(500, 1280).unwrap(), 640);
    assert!(grid(-1, 1280).is_err());
    assert!(grid(1000, 720).is_err());
    assert!(grid(0, 0).is_err());
}
