//! Desktop boundary; real OS implementations arrive in milestone 2.
use serde_json::Value;
use std::sync::Mutex;
#[derive(Debug, Clone)]
pub struct Screenshot {
    pub bytes: Vec<u8>,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
}
/// Object-safe desktop operations. Coordinates use the agent-visible screen.
pub trait Platform: Send + Sync {
    fn name(&self) -> &str;
    fn screen_size(&self) -> (u32, u32);
    fn screenshot(&self, include_cursor: bool) -> Result<Screenshot, String>;
    fn mouse_move(&self, args: &Value) -> Result<(), String>;
    fn click(&self, button: &str, count: u8, args: &Value) -> Result<(), String>;
    fn mouse_down(&self, args: &Value) -> Result<(), String>;
    fn mouse_up(&self, args: &Value) -> Result<(), String>;
    fn drag(&self, args: &Value) -> Result<(), String>;
    fn scroll(&self, args: &Value) -> Result<(), String>;
    fn key(&self, combo: &str) -> Result<(), String>;
    fn hold_key(&self, combo: &str, duration: f64) -> Result<(), String>;
    fn type_text(&self, text: &str) -> Result<(), String>;
}
#[derive(Default)]
pub struct MockPlatform {
    pub calls: Mutex<Vec<(String, Value)>>,
    pub available: bool,
}
impl MockPlatform {
    pub fn recording() -> Self {
        Self {
            available: true,
            ..Self::default()
        }
    }
    fn record(&self, name: &str, args: Value) -> Result<(), String> {
        if !self.available {
            return Err("desktop backend not available on this platform yet".into());
        }
        self.calls.lock().unwrap().push((name.into(), args));
        Ok(())
    }
}
pub type UnavailablePlatform = MockPlatform;
impl Platform for MockPlatform {
    fn name(&self) -> &str {
        if self.available {
            "mock"
        } else {
            "unavailable"
        }
    }
    fn screen_size(&self) -> (u32, u32) {
        (1280, 720)
    }
    fn screenshot(&self, include_cursor: bool) -> Result<Screenshot, String> {
        self.record(
            "screenshot",
            serde_json::json!({"include_cursor":include_cursor}),
        )?;
        Ok(Screenshot {
            bytes: vec![],
            mime_type: "image/png".into(),
            width: 1280,
            height: 720,
        })
    }
    fn mouse_move(&self, a: &Value) -> Result<(), String> {
        self.record("move_pointer", a.clone())
    }
    fn click(&self, b: &str, c: u8, a: &Value) -> Result<(), String> {
        self.record(&format!("click:{b}:{c}"), a.clone())
    }
    fn mouse_down(&self, a: &Value) -> Result<(), String> {
        self.record("left_mouse_down", a.clone())
    }
    fn mouse_up(&self, a: &Value) -> Result<(), String> {
        self.record("left_mouse_up", a.clone())
    }
    fn drag(&self, a: &Value) -> Result<(), String> {
        self.record("left_click_drag", a.clone())
    }
    fn scroll(&self, a: &Value) -> Result<(), String> {
        self.record("scroll", a.clone())
    }
    fn key(&self, s: &str) -> Result<(), String> {
        self.record("key", s.into())
    }
    fn hold_key(&self, s: &str, d: f64) -> Result<(), String> {
        self.record("hold_key", serde_json::json!({"keys":s,"duration":d}))
    }
    fn type_text(&self, s: &str) -> Result<(), String> {
        self.record("type_text", s.into())
    }
}
