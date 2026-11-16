//! Best-effort, unverified vs AWS. All schemas are PROVISIONAL vendor-derived.
mod anthropic;
mod google;
mod openai;
use crate::config::Config;
use serde_json::{json, Value};
#[derive(Debug, Clone, Copy)]
pub enum Interface {
    Anthropic20250124,
    Anthropic20251124,
    AnthropicToolset20260801,
    OpenaiV1,
    Google3x,
}
impl Interface {
    pub fn parse(s: &str) -> Result<Option<Self>, String> {
        Ok(match s {
            "" => None,
            "ANTHROPIC_COMPUTER_20250124" => Some(Self::Anthropic20250124),
            "ANTHROPIC_COMPUTER_20251124" => Some(Self::Anthropic20251124),
            "ANTHROPIC_COMPUTER_TOOLSET_20260801" => Some(Self::AnthropicToolset20260801),
            "OPENAI_COMPUTER_V1" => Some(Self::OpenaiV1),
            "GOOGLE_COMPUTER_USE_3X_DESKTOP_V1" => Some(Self::Google3x),
            _ => return Err("Unknown computer-use interface id".into()),
        })
    }
}

/// Only the standard desktop tools are replaced. Unknown future native tools stay.
pub fn retained(name: &str) -> bool {
    let Some(short) = name.strip_prefix(crate::PREFIX) else {
        return false;
    };
    !crate::is_pointer(short) && !matches!(short, "type_text" | "key" | "hold_key" | "screenshot")
}
impl Interface {
    fn actions(self) -> &'static [&'static str] {
        match self {
            Self::AnthropicToolset20260801 => anthropic::ACTIONS,
            Self::Google3x => google::ACTIONS,
            _ => &["computer"],
        }
    }
    fn input(self, name: &str) -> Value {
        match self {
            Self::OpenaiV1 => openai::schema(),
            Self::Google3x => google::schema(name),
            _ => anthropic::schema((name != "computer").then_some(name)),
        }
    }
    /// Convert only; native validation/execution remains authoritative.
    pub fn translate(
        self,
        name: &str,
        a: &Value,
        size: (u32, u32),
    ) -> Result<(String, Value), String> {
        if matches!(name, "wait" | "zoom" | "cursor_position") {
            return Err(rejected());
        }
        if !self.actions().contains(&name) {
            return Err("Unknown tool".into());
        }
        let action = if matches!(self, Self::OpenaiV1) {
            &a["type"]
        } else {
            &a["action"]
        };
        if action
            .as_str()
            .is_some_and(|a| matches!(a, "wait" | "zoom" | "cursor_position"))
        {
            return Err(rejected());
        }
        check(&self.input(name), a)?;
        match self {
            Self::OpenaiV1 => openai::translate(a),
            Self::Google3x => google::translate(name, a, size),
            _ => anthropic::translate(
                if name == "computer" {
                    string(a, "action")?
                } else {
                    name
                },
                a,
            ),
        }
    }
}
pub fn list(interface: Option<Interface>, c: &Config, modern: bool) -> Value {
    let mut r = crate::list(c, modern);
    let Some(i) = interface else { return r };
    let tools = r["tools"].as_array_mut().unwrap();
    tools.retain(|t| retained(t["name"].as_str().unwrap()));
    for name in i.actions() {
        // Single computer tools can expose screenshots even with input disabled;
        // execution still checks each translated action's permissions.
        if !c.agent_access.computer_input
            && (!c.agent_access.computer_vision
                || !matches!(*name, "computer" | "screenshot" | "take_screenshot"))
        {
            continue;
        }
        if matches!(*name, "screenshot" | "take_screenshot") && !c.agent_access.computer_vision {
            continue;
        }
        let mut schema = i.input(name);
        if modern {
            schema["properties"]["mcpSessionId"] = json!({"type":"string"});
            schema["required"]
                .as_array_mut()
                .unwrap()
                .push(json!("mcpSessionId"));
        }
        tools.push(json!({"name":name,"description":"Computer-use adapter: best-effort, unverified vs AWS. PROVISIONAL vendor-derived schema, not AWS-confirmed.","inputSchema":schema}));
    }
    r
}
fn rejected() -> String {
    "Unsupported action; use agentaccess___wait to pause".into()
}
fn string<'a>(a: &'a Value, key: &str) -> Result<&'a str, String> {
    a[key]
        .as_str()
        .ok_or_else(|| format!("{key} must be a string"))
}
fn integer(a: &Value, key: &str) -> Result<i64, String> {
    a[key]
        .as_i64()
        .ok_or_else(|| format!("{key} must be an integer"))
}
fn point(a: &Value, key: &str) -> Result<(i64, i64), String> {
    let p = a[key]
        .as_array()
        .filter(|p| p.len() == 2)
        .ok_or_else(|| format!("{key} must contain two integers"))?;
    Ok((
        p[0].as_i64().ok_or("invalid coordinate")?,
        p[1].as_i64().ok_or("invalid coordinate")?,
    ))
}
// PROVISIONAL: preserve xdotool names; normalize a small vendor alias vocabulary.
fn keys(s: &str) -> String {
    s.split('+')
        .map(|k| match k.to_ascii_lowercase().as_str() {
            "control" | "ctrl" => "ctrl".into(),
            "command" | "cmd" | "meta" | "win" | "super" => "super".into(),
            "option" | "alt" => "alt".into(),
            "shift" => "shift".into(),
            "enter" | "return" => "Return".into(),
            "esc" | "escape" => "Escape".into(),
            "arrowup" => "Up".into(),
            "arrowdown" => "Down".into(),
            "arrowleft" => "Left".into(),
            "arrowright" => "Right".into(),
            _ => k.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("+")
}
fn joined(a: &Value, field: &str) -> Result<String, String> {
    let values = a[field]
        .as_array()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("{field} must be a nonempty string array"))?;
    let parts = values
        .iter()
        .map(|v| v.as_str().filter(|s| !s.is_empty()).ok_or("invalid key"))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(keys(&parts.join("+")))
}
fn direction(s: &str) -> Result<&'static str, String> {
    match s.to_ascii_lowercase().as_str() {
        "up" => Ok("Up"),
        "down" => Ok("Down"),
        "left" => Ok("Left"),
        "right" => Ok("Right"),
        _ => Err("invalid scroll_direction".into()),
    }
}
fn notches(n: i64) -> Result<i64, String> {
    n.checked_mul(120)
        .ok_or_else(|| "scroll amount overflow".into())
}
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
// Basic adapter-shape checks, followed by the existing native semantic validation.
fn check(schema: &Value, a: &Value) -> Result<(), String> {
    let o = a.as_object().ok_or("arguments must be an object")?;
    for field in schema["required"].as_array().unwrap() {
        if !o.contains_key(field.as_str().unwrap()) {
            return Err(format!("missing {}", field.as_str().unwrap()));
        }
    }
    for (k, v) in o {
        if k == "mcpSessionId" {
            continue;
        }
        let rule = schema["properties"]
            .get(k)
            .ok_or_else(|| format!("unknown argument {k}"))?;
        let valid = match rule["type"].as_str().unwrap_or("") {
            "string" => v.is_string(),
            "integer" => v.as_i64().is_some(),
            "number" => v.as_f64().is_some(),
            "array" => v.is_array(),
            "boolean" => v.is_boolean(),
            _ => true,
        };
        if !valid
            || rule["enum"]
                .as_array()
                .is_some_and(|choices| !choices.contains(v))
        {
            return Err(format!("invalid {k}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provisional_units_and_key_names() {
        assert_eq!(notches(0).unwrap(), 0);
        assert_eq!(notches(2).unwrap(), 240);
        assert!(notches(i64::MAX).is_err());
        for (input, expected) in [
            ("uP", "Up"),
            ("DOWN", "Down"),
            ("left", "Left"),
            ("right", "Right"),
        ] {
            assert_eq!(direction(input).unwrap(), expected);
        }
        assert!(direction("diagonal").is_err());
        for (input, expected) in [
            ("Control+Return", "ctrl+Return"),
            ("CMD+Enter", "super+Return"),
            ("Option+Esc", "alt+Escape"),
            ("Meta+ArrowUp", "super+Up"),
            ("Win+ArrowDown", "super+Down"),
            ("Shift+ArrowLeft", "shift+Left"),
            ("CTRL+ArrowRight", "ctrl+Right"),
            ("super+F4", "super+F4"),
            ("unknown_key", "unknown_key"),
        ] {
            assert_eq!(keys(input), expected);
        }
        assert_eq!(
            joined(&json!({"keys":["CTRL","SHIFT","a"]}), "keys").unwrap(),
            "ctrl+shift+a"
        );
    }
}
