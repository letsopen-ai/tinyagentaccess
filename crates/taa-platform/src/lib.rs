//! Typed, blocking desktop boundary. All positions are primary-display physical pixels.
#[cfg(not(feature = "test-support"))]
mod input;
#[cfg(feature = "test-support")]
pub mod input;
pub mod keys;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "macos", test))]
mod macos_capture;
#[cfg(any(target_os = "macos", test))]
mod macos_keys;
pub mod pixels;
#[cfg(target_os = "linux")]
mod wayland;
#[cfg(windows)]
mod windows;
#[cfg(target_os = "linux")]
pub mod x11;
pub use keys::{Key, KeyCombo};
pub use pixels::{Frame, Point};
#[cfg(any(test, feature = "test-support"))]
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Operation {
    Move(Point),
    /// Release the left button at the current cursor position, without motion.
    ReleaseLeft,
    Click(Point, Button, u8, KeyCombo),
    Button(Point, bool, KeyCombo),
    Drag(Point, Point),
    Scroll(Point, ScrollDirection, i64, KeyCombo),
    Key(KeyCombo, Duration),
    Text(String),
}
/// Implementations serialize their native connection internally; callers also serialize dispatch.
pub trait Platform: Send + Sync {
    fn is_stopping(&self) -> bool {
        false
    }

    fn name(&self) -> &str;
    /// Cached physical dimensions; must never prompt or wait for consent.
    fn screen_size(&self) -> (u32, u32);
    fn capture(&self) -> Result<Frame, String>;
    fn capture_geometry(&self) -> Result<(u32, u32), String> {
        Ok(self.screen_size())
    }
    fn perform(&self, op: Operation) -> Result<(), String>;
}
/// Local safety cap: at most 100 discrete notches per operation.
pub const MAX_SCROLL_TICKS: i64 = 12_000;
pub fn notches(ticks: i64) -> u32 {
    ((ticks.max(1) as u64 + 60) / 120)
        .max(1)
        .min(u32::MAX as u64) as u32
}
pub fn detect(linux_backend: &str) -> Arc<dyn Platform> {
    let _ = linux_backend;
    #[cfg(target_os = "linux")]
    {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE").is_ok_and(|s| s == "wayland");
        if linux_backend == "wayland" || (linux_backend == "auto" && wayland) {
            return Arc::new(wayland::Wayland::default());
        }
        if linux_backend == "x11" || std::env::var_os("DISPLAY").is_some() {
            return Arc::new(
                std::env::var("DISPLAY")
                    .ok()
                    .and_then(|d| x11::X11::connect(&d).ok())
                    .unwrap_or_default(),
            );
        }
    }
    #[cfg(target_os = "macos")]
    {
        return Arc::new(macos::Mac::new());
    }
    #[cfg(windows)]
    {
        return Arc::new(windows::Windows::new());
    }
    #[allow(unreachable_code)]
    {
        let _ = linux_backend;
        Arc::new(NoDisplay)
    }
}
struct NoDisplay;
impl Platform for NoDisplay {
    fn name(&self) -> &str {
        "unavailable"
    }
    fn screen_size(&self) -> (u32, u32) {
        (1280, 720)
    }
    fn capture(&self) -> Result<Frame, String> {
        if cfg!(target_os = "linux") {
            Err("no graphical session: DISPLAY/WAYLAND_DISPLAY not set; run inside a desktop session".into())
        } else {
            Err("desktop backend not available on this platform yet".into())
        }
    }
    fn perform(&self, _: Operation) -> Result<(), String> {
        self.capture().map(|_| ())
    }
}
#[cfg(any(test, feature = "test-support"))]
mod mock {
    use super::*;
    use std::sync::Mutex;
    #[derive(Default)]
    pub struct MockPlatform {
        pub calls: Mutex<Vec<(String, Value)>>,
        pub operations: Mutex<Vec<Operation>>,
        pub available: bool,
    }
    pub type UnavailablePlatform = MockPlatform;
    impl MockPlatform {
        pub fn recording() -> Self {
            Self {
                available: true,
                ..Self::default()
            }
        }
        fn record(&self, n: String, v: Value) -> Result<(), String> {
            if !self.available {
                return Err("desktop backend not available on this platform yet".into());
            }
            self.calls.lock().unwrap().push((n, v));
            Ok(())
        }
    }
    impl Platform for MockPlatform {
        fn name(&self) -> &str {
            if self.available {
                "mock"
            } else {
                "unavailable"
            }
        }
        fn screen_size(&self) -> (u32, u32) {
            (1920, 1080)
        }
        fn capture(&self) -> Result<Frame, String> {
            self.record("screenshot".into(), json!({}))?;
            Ok(Frame::solid(1920, 1080, [40, 80, 120, 255]))
        }
        fn perform(&self, op: Operation) -> Result<(), String> {
            let name = match &op {
                Operation::ReleaseLeft => "left_mouse_up".into(),
                Operation::Move(_) => "move_pointer".into(),
                Operation::Click(_, b, n, _) => format!(
                    "click:{}:{n}",
                    match b {
                        Button::Left => "left",
                        Button::Middle => "middle",
                        Button::Right => "right",
                    }
                ),
                Operation::Button(_, true, _) => "left_mouse_down".into(),
                Operation::Button(_, false, _) => "left_mouse_up".into(),
                Operation::Drag(..) => "left_click_drag".into(),
                Operation::Scroll(..) => "scroll".into(),
                Operation::Key(_, d) => if d.is_zero() { "key" } else { "hold_key" }.into(),
                Operation::Text(_) => "type_text".into(),
            };
            self.record(name, json!({}))?;
            self.operations.lock().unwrap().push(op);
            Ok(())
        }
    }
}
#[cfg(any(test, feature = "test-support"))]
pub use mock::{MockPlatform, UnavailablePlatform};
#[cfg(test)]
mod tests {
    #[test]
    fn ticks() {
        for (t, n) in [(1, 1), (60, 1), (120, 1), (179, 1), (180, 2), (360, 3)] {
            assert_eq!(super::notches(t), n);
        }
    }
}
