//! ScreenCaptureKit capture with CoreGraphics geometry, conversion and input.
use crate::macos_capture::{self, permission};
use crate::macos_keys::ascii;
use crate::{
    input::{self, Input, EVENT_DELAY},
    pixels::{Frame, Point},
    Button, Key, Operation, Platform, ScrollDirection,
};
use std::{
    ffi::c_void,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    thread::sleep,
};
type Ref = *mut c_void;
#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}
#[repr(C)]
struct CGSize {
    width: f64,
    height: f64,
}
#[repr(C)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGDisplayCopyDisplayMode(d: u32) -> Ref;
    fn CGDisplayModeGetPixelWidth(m: Ref) -> usize;
    fn CGDisplayModeGetPixelHeight(m: Ref) -> usize;
    fn CGDisplayModeRelease(m: Ref);
    fn CGDisplayBounds(d: u32) -> CGRect;
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGImageGetWidth(i: Ref) -> usize;
    fn CGImageGetHeight(i: Ref) -> usize;
    fn CGColorSpaceCreateDeviceRGB() -> Ref;
    fn CGColorSpaceRelease(c: Ref);
    fn CGBitmapContextCreate(
        data: *mut c_void,
        w: usize,
        h: usize,
        bits: usize,
        row: usize,
        space: Ref,
        info: u32,
    ) -> Ref;
    fn CGContextDrawImage(c: Ref, r: CGRect, i: Ref);
    fn CGContextRelease(c: Ref);
    fn CGEventCreate(source: Ref) -> Ref;
    fn CGEventGetLocation(e: Ref) -> CGPoint;
    fn CGEventCreateMouseEvent(s: Ref, kind: u32, p: CGPoint, button: u32) -> Ref;
    fn CGEventCreateKeyboardEvent(s: Ref, key: u16, down: bool) -> Ref;
    fn CGEventKeyboardSetUnicodeString(e: Ref, len: usize, text: *const u16);
    #[cfg(test)]
    fn CGEventGetIntegerValueField(e: Ref, field: u32) -> i64;
    fn CGEventSetIntegerValueField(e: Ref, field: u32, value: i64);
    fn CGEventSetFlags(e: Ref, flags: u64);
    fn CGEventSetLocation(e: Ref, p: CGPoint);
    fn CGEventPost(tap: u32, e: Ref);
    fn CGEventCreateScrollWheelEvent2(
        s: Ref,
        units: u32,
        count: u32,
        wheel1: i32,
        wheel2: i32,
        wheel3: i32,
    ) -> Ref;
}
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
}
// doctor-preflight-begin
// No-prompt queries only; used by `preflight::macos` for `tinyagentaccess doctor`.
pub(crate) fn preflight_screen_recording() -> bool {
    // SAFETY: query-only; never shows a consent prompt.
    unsafe { CGPreflightScreenCaptureAccess() }
}
pub(crate) fn preflight_accessibility() -> bool {
    // SAFETY: the plain form never prompts.
    unsafe { AXIsProcessTrusted() }
}
// doctor-preflight-end
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(p: Ref);
}
#[derive(Default)]
pub struct Mac {
    size: AtomicU64,
    lock: Mutex<Option<Desktop>>,
    capture_lock: Mutex<()>,
}
impl Mac {
    pub fn new() -> Self {
        let p = Self::default();
        let _ = p.capture_geometry();
        p
    }
}
fn post(e: Ref) -> Result<(), String> {
    if e.is_null() {
        return Err("CoreGraphics could not create an input event".into());
    }
    unsafe {
        CGEventPost(0, e);
        CFRelease(e);
    }
    Ok(())
}
fn keycode(k: Key) -> Option<u16> {
    Some(match k.0 {
        0xffe3 => 59,
        0xffe1 => 56,
        0xffe9 => 58,
        0xffeb => 55,
        0x11000000 => 63,
        0xff0d => 36,
        0xff09 => 48,
        0xff1b => 53,
        32 => 49,
        0xff08 => 51,
        0xffff => 117,
        0xff63 => 114,
        0xff50 => 115,
        0xff57 => 119,
        0xff55 => 116,
        0xff56 => 121,
        0xff51 => 123,
        0xff52 => 126,
        0xff53 => 124,
        0xff54 => 125,
        0xffe5 => 57,
        0xff7f => 71,
        0xff61 => 105,
        0xff14 => 107,
        0xff13 => 113,
        0xff67 => 110,
        0xff80 => 49,
        0xff89 => 48,
        0xff91 => 122,
        0xff92 => 120,
        0xff93 => 99,
        0xff94 => 118,
        0xffac => 65,
        0xff95 => 115,
        0xff96 => 123,
        0xff97 => 126,
        0xff98 => 124,
        0xff99 => 125,
        0xff9a => 116,
        0xff9b => 121,
        0xff9c => 119,
        0xff9d => 71,
        0xff9e => 114,
        0xff9f => 117,
        0xff8d => 76,
        0xffab => 69,
        0xffad => 78,
        0xffaa => 67,
        0xffaf => 75,
        0xffae => 65,
        0xffbd => 81,
        n @ 0xffb0..=0xffb9 => [82, 83, 84, 85, 86, 87, 88, 89, 91, 92][(n - 0xffb0) as usize],
        n @ 0xffbe..=0xffd1 => [
            122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111, 105, 107, 113, 106, 64, 79, 80,
            90,
        ][(n - 0xffbe) as usize],
        _ => return None,
    })
}
fn physical_size(d: u32) -> Result<(u32, u32), String> {
    unsafe {
        let mode = CGDisplayCopyDisplayMode(d);
        if mode.is_null() {
            return Err("CoreGraphics: no current display mode".into());
        }
        let size = (
            CGDisplayModeGetPixelWidth(mode) as u32,
            CGDisplayModeGetPixelHeight(mode) as u32,
        );
        CGDisplayModeRelease(mode);
        if size.0 == 0 || size.1 == 0 {
            return Err("CoreGraphics: invalid physical display dimensions".into());
        }
        Ok(size)
    }
}
struct Desktop {
    position: CGPoint,
    flags: u64,
    left_down: bool,
}
impl Desktop {
    fn wheel_event(&self, vertical: i32, horizontal: i32) -> Ref {
        unsafe {
            // SDK CGEvent.h: fixed-argument API since macOS 10.13; line units = 1.
            let e =
                CGEventCreateScrollWheelEvent2(std::ptr::null_mut(), 1, 2, vertical, horizontal, 0);
            if !e.is_null() {
                CGEventSetLocation(e, self.position);
                CGEventSetFlags(e, self.flags);
            }
            e
        }
    }

    fn new() -> Self {
        unsafe {
            let d = CGMainDisplayID();
            let b = CGDisplayBounds(d);
            let e = CGEventCreate(std::ptr::null_mut());
            let position = if e.is_null() {
                b.origin
            } else {
                let p = CGEventGetLocation(e);
                CFRelease(e);
                p
            };
            Self {
                position,
                flags: 0,
                left_down: false,
            }
        }
    }
    fn mouse(&mut self, kind: u32, b: u32, count: u8) -> Result<(), String> {
        unsafe {
            let e = CGEventCreateMouseEvent(std::ptr::null_mut(), kind, self.position, b);
            if !e.is_null() {
                CGEventSetIntegerValueField(e, 1, count as i64);
                CGEventSetFlags(e, self.flags);
            }
            post(e)
        }
    }
}
impl Platform for Mac {
    fn name(&self) -> &str {
        macos_capture::backend_name(macos_capture::available())
    }
    fn screen_size(&self) -> (u32, u32) {
        let s = self.size.load(Ordering::Relaxed);
        if s == 0 {
            (1280, 720)
        } else {
            ((s >> 32) as u32, s as u32)
        }
    }
    fn capture_geometry(&self) -> Result<(u32, u32), String> {
        let (w, h) = unsafe {
            let d = CGMainDisplayID();
            physical_size(d)?
        };
        self.size
            .store(((w as u64) << 32) | h as u64, Ordering::Relaxed);
        Ok((w, h))
    }
    fn capture(&self) -> Result<Frame, String> {
        let _guard = self.capture_lock.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            if !CGPreflightScreenCaptureAccess() {
                return Err(permission(true));
            }
            let d = CGMainDisplayID();
            let owned_image = macos_capture::capture(d, physical_size(d)?)?;
            let image = owned_image.0;
            let w = CGImageGetWidth(image);
            let h = CGImageGetHeight(image);
            if physical_size(d) != Ok((w as u32, h as u32)) {
                return Err("CoreGraphics: capture and current physical display mode disagree; retry after display changes settle".into());
            }
            self.size
                .store(((w as u64) << 32) | h as u64, Ordering::Relaxed);
            let mut f = Frame::solid(w as u32, h as u32, [0, 0, 0, 255]);
            let space = CGColorSpaceCreateDeviceRGB();
            let context =
                CGBitmapContextCreate(f.rgba.as_mut_ptr().cast(), w, h, 8, w * 4, space, 0x4001);
            CGColorSpaceRelease(space);
            if context.is_null() {
                return Err("CoreGraphics bitmap context creation failed".into());
            }
            CGContextDrawImage(
                context,
                CGRect {
                    origin: CGPoint { x: 0., y: 0. },
                    size: CGSize {
                        width: w as f64,
                        height: h as f64,
                    },
                },
                image,
            );
            CGContextRelease(context);
            drop(owned_image);
            let b = CGDisplayBounds(d);
            f.logical_size = (b.size.width as u32, b.size.height as u32);
            let e = CGEventCreate(std::ptr::null_mut());
            if !e.is_null() {
                let p = CGEventGetLocation(e);
                CFRelease(e);
                f.cursor = Some(crate::pixels::arrow(Point {
                    x: ((p.x - b.origin.x) * w as f64 / b.size.width).round() as i32,
                    y: ((p.y - b.origin.y) * h as f64 / b.size.height).round() as i32,
                }));
            }
            Ok(f)
        }
    }
    fn perform(&self, op: Operation) -> Result<(), String> {
        if !unsafe { AXIsProcessTrusted() } {
            return Err(permission(false));
        }
        let mut guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(
            op,
            Operation::Move(_)
                | Operation::Click(..)
                | Operation::Button(..)
                | Operation::Drag(..)
                | Operation::Scroll(..)
        ) && physical_size(unsafe { CGMainDisplayID() })? != self.screen_size()
        {
            return Err("CoreGraphics: display dimensions changed during pointer operation; retry from a new screenshot".into());
        }
        input::perform(guard.get_or_insert_with(Desktop::new), op)
    }
}
impl Input for Desktop {
    fn release_left(&mut self) -> Result<(), String> {
        unsafe {
            let event = CGEventCreate(std::ptr::null_mut());
            if event.is_null() {
                return Err("CoreGraphics: could not query cursor position for release".into());
            }
            self.position = CGEventGetLocation(event);
            CFRelease(event);
        }
        self.button(Button::Left, false, 1)
    }

    fn motion(&mut self, p: Point) -> Result<(), String> {
        let d = unsafe { CGMainDisplayID() };
        let b = unsafe { CGDisplayBounds(d) };
        let (w, h) = physical_size(d)?;
        let (x, y) = crate::pixels::desktop_point(
            p,
            (w, h),
            (b.size.width, b.size.height),
            (b.origin.x, b.origin.y),
        );
        self.position = CGPoint { x, y };
        self.mouse(if self.left_down { 6 } else { 5 }, 0, 0)
    }
    fn button(&mut self, b: Button, down: bool, count: u8) -> Result<(), String> {
        let (kind, button) = match (b, down) {
            (Button::Left, true) => (1, 0),
            (Button::Left, false) => (2, 0),
            (Button::Right, true) => (3, 1),
            (Button::Right, false) => (4, 1),
            (Button::Middle, true) => (25, 2),
            (Button::Middle, false) => (26, 2),
        };
        self.mouse(kind, button, count)?;
        if b == Button::Left {
            self.left_down = down;
        }
        Ok(())
    }
    fn key(&mut self, k: Key, down: bool) -> Result<(), String> {
        let flag = match k.0 {
            0xffe3 => 1 << 18,
            0xffe1 => 1 << 17,
            0xffe9 => 1 << 19,
            0xffeb => 1 << 20,
            0x11000000 => 1 << 23,
            _ => 0,
        };
        if down {
            self.flags |= flag;
        } else {
            self.flags &= !flag;
        }
        let (code, shift) = if let Some(c) = keycode(k) {
            (c, false)
        } else {
            if k.0 >= 0xff00 && k.0 < 0x01000000 || k.0 > 0x0100ffff {
                return Err(format!(
                    "macOS: unsupported special key {}",
                    crate::keys::label(k)
                ));
            }
            let c = char::from_u32(k.0 & 0xffffff).ok_or("macOS: invalid character key")?;
            ascii(c)
                .ok_or_else(|| format!("macOS: key {c} has no hardware keycode; use type_text"))?
        };
        unsafe {
            let e = CGEventCreateKeyboardEvent(std::ptr::null_mut(), code, down);
            if !e.is_null() {
                CGEventSetFlags(e, self.flags | if shift { 1 << 17 } else { 0 });
            }
            post(e)
        }
    }
    fn scroll(&mut self, d: ScrollDirection, t: i64) -> Result<(), String> {
        if t == 0 {
            return Ok(());
        }
        let n = crate::notches(t.saturating_abs()).min(i32::MAX as u32) as i32
            * if t < 0 { -1 } else { 1 };
        let (v, h) = match d {
            ScrollDirection::Up => (n, 0),
            ScrollDirection::Down => (-n, 0),
            ScrollDirection::Left => (0, n),
            ScrollDirection::Right => (0, -n),
        };
        post(self.wheel_event(v, h))
    }
    fn text(&mut self, t: &str) -> Result<(), String> {
        for c in t.chars() {
            if c == '\n' || c == '\t' {
                let k = if c == '\n' { Key(0xff0d) } else { Key(0xff09) };
                let down = self.key(k, true);
                down.and(self.key(k, false))?;
            } else {
                let mut buf = [0; 2];
                let units = c.encode_utf16(&mut buf);
                for down in [true, false] {
                    unsafe {
                        let e = CGEventCreateKeyboardEvent(std::ptr::null_mut(), 0, down);
                        if !e.is_null() {
                            CGEventKeyboardSetUnicodeString(e, units.len(), units.as_ptr());
                        }
                        post(e)?;
                    }
                }
            }
            sleep(EVENT_DELAY);
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    // Regression diagnostic for the construction-location mismatch observed on macOS.
    // No event is posted and no cursor movement or capture is requested.
    #[test]
    #[ignore = "requires macOS CoreGraphics; checks construction only, not AppKit delivery"]
    fn wheel_location_matches_requested_position() {
        let mut desktop = Desktop::new();
        // Deliberately differ from the sampled cursor location, so inheriting
        // global cursor state cannot accidentally satisfy the target assertion.
        desktop.position.x += 37.0;
        desktop.position.y += 53.0;
        let e = desktop.wheel_event(-1, 0);
        assert!(!e.is_null(), "CoreGraphics could not create wheel event");
        let actual = unsafe {
            let p = CGEventGetLocation(e);
            // SDK CGEventTypes.h: kCGScrollWheelEventDeltaAxis1/2 = 11/12.
            let axes = (
                CGEventGetIntegerValueField(e, 11),
                CGEventGetIntegerValueField(e, 12),
            );
            CFRelease(e);
            (p, axes)
        };
        assert_eq!(
            actual.1,
            (-1, 0),
            "wheel axes must equal requested line deltas"
        );
        assert_eq!(
            (actual.0.x, actual.0.y),
            (desktop.position.x, desktop.position.y),
            "wheel location must equal the requested CGEvent position"
        );
    }

    #[test]
    fn named_mapping_audit() {
        for name in crate::keys::SPECIAL_NAMES {
            let key = Key::parse(name).unwrap();
            assert_eq!(
                keycode(key).is_some(),
                !matches!(*name, "volumeup" | "volumedown" | "mute" | "play"),
                "{name}"
            );
        }
        for n in 1..=24 {
            assert_eq!(
                keycode(Key::parse(&format!("F{n}")).unwrap()).is_some(),
                n <= 20
            );
        }
        for n in 0..=9 {
            assert!(keycode(Key::parse(&format!("KP_{n}")).unwrap()).is_some());
        }
    }
    #[test]
    fn tables() {
        assert_eq!(keycode(Key(0xffeb)), Some(55));
        assert_eq!(ascii('a'), Some((0, false)));
        assert_eq!(ascii('b'), Some((11, false)));
        assert_eq!(ascii('p'), Some((35, false)));
        assert_eq!(ascii('T'), Some((17, true)));
        for c in ' '..='~' {
            assert!(ascii(c).is_some(), "{c}");
        }
        for (c, code) in [('j', 38), ('k', 40), ('l', 37), ('n', 45), ('m', 46)] {
            assert_eq!(ascii(c), Some((code, false)));
            assert_eq!(ascii(c.to_ascii_uppercase()), Some((code, true)));
        }
        assert!(permission(true).contains("Screen Recording"));
    }
    #[test]
    #[ignore = "requires Screen Recording permission and interactive macOS desktop"]
    fn gui() {
        Mac::default().capture().unwrap();
    }
}
