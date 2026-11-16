//! Primary-display GDI capture and physical-coordinate SendInput.
use crate::{
    input::{self, Input, EVENT_DELAY},
    pixels::{Frame, Point},
    Button, Key, Operation, Platform, ScrollDirection,
};
use std::{
    mem::{size_of, zeroed},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    thread::sleep,
};
use windows_sys::Win32::{
    Graphics::Gdi::*,
    UI::{HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};
#[derive(Default)]
pub struct Windows {
    size: AtomicU64,
    lock: Mutex<()>,
}
impl Windows {
    pub fn new() -> Self {
        let p = Self::default();
        let _ = p.capture_geometry();
        p
    }
}
fn failure() -> String {
    format!("Windows desktop: {}; run in an unlocked interactive desktop at the same integrity level as the target application",std::io::Error::last_os_error())
}
fn aware() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}
fn send(input: INPUT) -> Result<(), String> {
    if unsafe { SendInput(1, &input, size_of::<INPUT>() as i32) } == 1 {
        Ok(())
    } else {
        Err(failure())
    }
}
fn mouse(dx: i32, dy: i32, data: u32, flags: u32) -> Result<(), String> {
    send(INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    })
}
fn keyboard(vk: u16, scan: u16, flags: u32) -> Result<(), String> {
    send(INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    })
}
fn vk(k: Key) -> Option<u16> {
    Some(match k.0 {
        0xffe3 => VK_CONTROL,
        0xffe1 => VK_SHIFT,
        0xffe9 => VK_MENU,
        0xffeb => VK_LWIN,
        0xff0d => VK_RETURN,
        0xff09 => VK_TAB,
        0xff1b => VK_ESCAPE,
        32 => VK_SPACE,
        0xff08 => VK_BACK,
        0xffff => VK_DELETE,
        0xff63 => VK_INSERT,
        0xff50 => VK_HOME,
        0xff57 => VK_END,
        0xff55 => VK_PRIOR,
        0xff56 => VK_NEXT,
        0xff51 => VK_LEFT,
        0xff52 => VK_UP,
        0xff53 => VK_RIGHT,
        0xff54 => VK_DOWN,
        0xffe5 => VK_CAPITAL,
        0xff7f => VK_NUMLOCK,
        0xff14 => VK_SCROLL,
        0xff61 => VK_SNAPSHOT,
        0xff13 => VK_PAUSE,
        0xff67 => VK_APPS,
        0xff80 => VK_SPACE,
        0xff89 => VK_TAB,
        0xff91..=0xff94 => VK_F1 + (k.0 - 0xff91) as u16,
        0xffac => VK_SEPARATOR,
        0xff95 => VK_HOME,
        0xff96 => VK_LEFT,
        0xff97 => VK_UP,
        0xff98 => VK_RIGHT,
        0xff99 => VK_DOWN,
        0xff9a => VK_PRIOR,
        0xff9b => VK_NEXT,
        0xff9c => VK_END,
        0xff9d => VK_CLEAR,
        0xff9e => VK_INSERT,
        0xff9f => VK_DELETE,
        0xff8d => VK_RETURN,
        0xffab => VK_ADD,
        0xffad => VK_SUBTRACT,
        0xffaa => VK_MULTIPLY,
        0xffaf => VK_DIVIDE,
        0xffae => VK_DECIMAL,
        0xffbd => VK_OEM_PLUS,
        0x1008ff13 => VK_VOLUME_UP,
        0x1008ff11 => VK_VOLUME_DOWN,
        0x1008ff12 => VK_VOLUME_MUTE,
        0x1008ff14 => VK_MEDIA_PLAY_PAUSE,
        n @ 0xffbe..=0xffd5 => VK_F1 + (n - 0xffbe) as u16,
        n @ 0xffb0..=0xffb9 => VK_NUMPAD0 + (n - 0xffb0) as u16,
        _ => return None,
    })
}
struct Desktop;
impl Platform for Windows {
    fn name(&self) -> &str {
        "windows-sendinput-gdi"
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
        aware();
        let (w, h) = unsafe {
            (
                GetSystemMetrics(SM_CXSCREEN).max(1) as u32,
                GetSystemMetrics(SM_CYSCREEN).max(1) as u32,
            )
        };
        self.size
            .store(((w as u64) << 32) | h as u64, Ordering::Relaxed);
        Ok((w, h))
    }
    fn capture(&self) -> Result<Frame, String> {
        let _guard = self.lock.lock().map_err(|e| e.to_string())?;
        aware();
        unsafe {
            let (w, h) = self.capture_geometry()?;
            let screen = GetDC(std::ptr::null_mut());
            if screen.is_null() {
                return Err(failure());
            }
            let mem = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, w as i32, h as i32);
            if mem.is_null() || bitmap.is_null() {
                if !mem.is_null() {
                    DeleteDC(mem);
                }
                if !bitmap.is_null() {
                    DeleteObject(bitmap);
                }
                ReleaseDC(std::ptr::null_mut(), screen);
                return Err(failure());
            }
            let old = SelectObject(mem, bitmap);
            let copied = BitBlt(
                mem,
                0,
                0,
                w as i32,
                h as i32,
                screen,
                0,
                0,
                SRCCOPY | CAPTUREBLT,
            );
            SelectObject(mem, old);
            let mut info: BITMAPINFO = zeroed();
            info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = w as i32;
            info.bmiHeader.biHeight = -(h as i32);
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            let mut f = Frame::solid(w, h, [0, 0, 0, 255]);
            let lines = GetDIBits(
                mem,
                bitmap,
                0,
                h,
                f.rgba.as_mut_ptr().cast(),
                &mut info,
                DIB_RGB_COLORS,
            );
            DeleteObject(bitmap);
            DeleteDC(mem);
            ReleaseDC(std::ptr::null_mut(), screen);
            if copied == 0 || lines != h as i32 {
                return Err(failure());
            }
            for p in f.rgba.as_chunks_mut::<4>().0.iter_mut() {
                p.swap(0, 2);
                p[3] = 255;
            }
            let mut cursor: CURSORINFO = zeroed();
            cursor.cbSize = size_of::<CURSORINFO>() as u32;
            if GetCursorInfo(&mut cursor) != 0 && cursor.flags == CURSOR_SHOWING {
                f.cursor = Some(crate::pixels::arrow(Point {
                    x: cursor.ptScreenPos.x,
                    y: cursor.ptScreenPos.y,
                }));
            }
            Ok(f)
        }
    }
    fn perform(&self, op: Operation) -> Result<(), String> {
        let _guard = self.lock.lock().map_err(|e| e.to_string())?;
        aware();
        input::perform(&mut Desktop, op)
    }
}
impl Input for Desktop {
    fn motion(&mut self, p: Point) -> Result<(), String> {
        unsafe {
            let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            mouse(
                ((p.x - x) as f64 * 65535. / (w - 1).max(1) as f64).round() as i32,
                ((p.y - y) as f64 * 65535. / (h - 1).max(1) as f64).round() as i32,
                0,
                MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            )
        }
    }
    fn button(&mut self, b: Button, down: bool, _: u8) -> Result<(), String> {
        mouse(
            0,
            0,
            0,
            match (b, down) {
                (Button::Left, true) => MOUSEEVENTF_LEFTDOWN,
                (Button::Left, false) => MOUSEEVENTF_LEFTUP,
                (Button::Right, true) => MOUSEEVENTF_RIGHTDOWN,
                (Button::Right, false) => MOUSEEVENTF_RIGHTUP,
                (Button::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
                (Button::Middle, false) => MOUSEEVENTF_MIDDLEUP,
            },
        )
    }
    fn key(&mut self, k: Key, down: bool) -> Result<(), String> {
        let mut shift = false;
        let code = if let Some(v) = vk(k) {
            v
        } else {
            let c = char::from_u32(k.0 & 0x00ffffff).ok_or("Windows: invalid key")?;
            if c as u32 > 65535 {
                return Err("Windows: use type_text for supplementary Unicode characters".into());
            }
            let v = unsafe { VkKeyScanW(c as u16) };
            if v == -1 {
                return Err(format!("Windows: key {c} unavailable; use type_text"));
            }
            shift = v & 0x100 != 0;
            (v & 255) as u16
        };
        if down && shift {
            keyboard(VK_SHIFT, 0, 0)?;
        }
        let flags = if down { 0 } else { KEYEVENTF_KEYUP };
        let ext = if matches!(
            code,
            VK_LEFT
                | VK_RIGHT
                | VK_UP
                | VK_DOWN
                | VK_HOME
                | VK_END
                | VK_PRIOR
                | VK_NEXT
                | VK_INSERT
                | VK_DELETE
                | VK_LWIN
                | VK_APPS
        ) {
            KEYEVENTF_EXTENDEDKEY
        } else {
            0
        };
        let r = keyboard(code, 0, flags | ext);
        if !down && shift {
            r.and(keyboard(VK_SHIFT, 0, KEYEVENTF_KEYUP))
        } else {
            r
        }
    }
    fn scroll(&mut self, d: ScrollDirection, t: i64) -> Result<(), String> {
        let sign = if matches!(d, ScrollDirection::Down | ScrollDirection::Left) {
            -1i64
        } else {
            1
        };
        let ticks = t
            .saturating_mul(sign)
            .clamp(i32::MIN as i64, i32::MAX as i64) as i32;
        mouse(
            0,
            0,
            ticks as u32,
            if matches!(d, ScrollDirection::Up | ScrollDirection::Down) {
                MOUSEEVENTF_WHEEL
            } else {
                MOUSEEVENTF_HWHEEL
            },
        )
    }
    fn text(&mut self, t: &str) -> Result<(), String> {
        for c in t.chars() {
            if c == '\n' || c == '\t' {
                let v = if c == '\n' { VK_RETURN } else { VK_TAB };
                keyboard(v, 0, 0)?;
                keyboard(v, 0, KEYEVENTF_KEYUP)?;
            } else {
                for unit in c.encode_utf16(&mut [0; 2]) {
                    keyboard(0, *unit, KEYEVENTF_UNICODE)?;
                    keyboard(0, *unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)?;
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
    #[test]
    fn tables() {
        assert_eq!(vk(Key(0xffbe)), Some(VK_F1));
        assert_eq!(vk(Key(0xffd5)), Some(VK_F24));
    }
    #[test]
    #[ignore = "requires unlocked Windows interactive desktop"]
    fn gui() {
        Windows::default().capture().unwrap();
    }
}
