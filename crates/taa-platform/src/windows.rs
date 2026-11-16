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
fn aware() -> Result<(), String> {
    static INIT: std::sync::Once = std::sync::Once::new();
    unsafe {
        INIT.call_once(|| {
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        });
        if AreDpiAwarenessContextsEqual(
            GetThreadDpiAwarenessContext(),
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        ) == 0
        {
            return Err("Windows: PER_MONITOR_AWARE_V2 DPI context required; restart without a conflicting DPI override".into());
        }
    }
    Ok(())
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
                wScan: if vk == 0 {
                    scan
                } else {
                    unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) as u16 }
                },
                dwFlags: flags | extended(vk),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    })
}
fn extended(code: u16) -> u32 {
    if matches!(
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
    }
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
#[derive(Default)]
struct Desktop {
    pressed: std::collections::HashMap<u32, (u16, Vec<u16>)>,
}
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
        aware()?;
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
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        aware()?;
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
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        aware()?;
        let mut desktop = Desktop::default();
        let result = input::perform(&mut desktop, op);
        let keys = desktop.pressed.keys().copied().collect::<Vec<_>>();
        let mut cleanup = Ok(());
        for key in keys {
            let r = desktop.key(Key(key), false);
            if cleanup.is_ok() {
                cleanup = r;
            }
        }
        result.and(cleanup)
    }
}
impl Desktop {
    fn unheld_modifiers(&self, required: &[u16]) -> Vec<u16> {
        required
            .iter()
            .copied()
            .filter(|m| {
                !self
                    .pressed
                    .values()
                    .any(|(code, mods)| code == m || mods.contains(m))
            })
            .collect()
    }
}
fn required_modifiers(v: i16) -> Vec<u16> {
    [(0x100, VK_SHIFT), (0x200, VK_CONTROL), (0x400, VK_MENU)]
        .into_iter()
        .filter_map(|(bit, key)| (v & bit != 0).then_some(key))
        .collect()
}
// SendInput partitions 0..65535 into physical pixel bins. Aim at each bin's
// center, rather than its edge, to avoid one-pixel errors after quantization.
fn normalize_axis(pixel: i32, origin: i32, extent: i32) -> i32 {
    (((pixel as f64 - origin as f64 + 0.5) * 65536. / extent.max(1) as f64).floor() as i32)
        .clamp(0, 65535)
}
impl Input for Desktop {
    fn motion(&mut self, p: Point) -> Result<(), String> {
        unsafe {
            let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            mouse(
                normalize_axis(p.x, x, w),
                normalize_axis(p.y, y, h),
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
        if !down {
            if let Some((code, modifiers)) = self.pressed.get(&k.0).cloned() {
                let mut result = keyboard(code, 0, KEYEVENTF_KEYUP);
                for m in modifiers.into_iter().rev() {
                    let r = keyboard(m, 0, KEYEVENTF_KEYUP);
                    if result.is_ok() {
                        result = r;
                    }
                }
                if result.is_ok() {
                    self.pressed.remove(&k.0);
                }
                return result;
            }
            return Ok(());
        }
        let (code, required) = if let Some(v) = vk(k) {
            (v, vec![])
        } else {
            if k.0 >= 0xff00 && k.0 < 0x01000000 || k.0 >= 0x10000000 {
                return Err(format!(
                    "Windows: unsupported special key {}",
                    crate::keys::label(k)
                ));
            }
            let c = char::from_u32(k.0 & 0x00ffffff).ok_or("Windows: invalid key")?;
            if c as u32 > 65535 {
                return Err("Windows: use type_text for supplementary Unicode characters".into());
            }
            let v = unsafe { VkKeyScanW(c as u16) };
            if v == -1 {
                return Err(format!("Windows: key {c} unavailable; use type_text"));
            }
            ((v & 255) as u16, required_modifiers(v))
        };
        let owned = self.unheld_modifiers(&required);
        for (index, m) in owned.iter().enumerate() {
            if let Err(e) = keyboard(*m, 0, 0) {
                for m in owned[..=index].iter().rev() {
                    let _ = keyboard(*m, 0, KEYEVENTF_KEYUP);
                }
                return Err(e);
            }
        }
        self.pressed.insert(k.0, (code, owned));
        keyboard(code, 0, 0)
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
                let down = keyboard(v, 0, 0);
                down.and(keyboard(v, 0, KEYEVENTF_KEYUP))?;
            } else {
                for unit in c.encode_utf16(&mut [0; 2]) {
                    let down = keyboard(0, *unit, KEYEVENTF_UNICODE);
                    down.and(keyboard(0, *unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP))?;
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
    fn implicit_modifiers_preserve_explicit_and_shared_ownership() {
        let mut d = Desktop::default();
        d.pressed.insert(0xffe1, (VK_SHIFT, vec![]));
        assert_eq!(
            d.unheld_modifiers(&required_modifiers(0x741)),
            vec![VK_CONTROL, VK_MENU]
        );
        d.pressed.insert(65, (65, vec![VK_CONTROL, VK_MENU]));
        assert!(d.unheld_modifiers(&required_modifiers(0x741)).is_empty());
        assert_eq!(extended(VK_LEFT), KEYEVENTF_EXTENDEDKEY);
    }
    #[test]
    fn absolute_pixel_bins() {
        for (origin, width) in [(-1920, 3840), (0, 1920), (0, 2560), (0, 3840)] {
            for offset in [0, 1, width / 2, width - 2, width - 1] {
                let n = normalize_axis(origin + offset, origin, width);
                assert_eq!(n as i64 * width as i64 / 65536, offset as i64);
            }
        }
    }
    #[test]
    fn named_mapping_audit() {
        for name in crate::keys::SPECIAL_NAMES {
            assert_eq!(
                vk(Key::parse(name).unwrap()).is_some(),
                *name != "fn",
                "{name}"
            );
        }
        for n in 1..=24 {
            assert!(vk(Key::parse(&format!("F{n}")).unwrap()).is_some());
        }
        for n in 0..=9 {
            assert!(vk(Key::parse(&format!("KP_{n}")).unwrap()).is_some());
        }
    }
    #[test]
    fn tables() {
        assert_eq!(
            required_modifiers(0x741),
            vec![VK_SHIFT, VK_CONTROL, VK_MENU]
        );
        assert_eq!(required_modifiers(0x641), vec![VK_CONTROL, VK_MENU]);
        assert_eq!(vk(Key(0xffbe)), Some(VK_F1));
        assert_eq!(vk(Key(0xffd5)), Some(VK_F24));
    }
    #[test]
    #[ignore = "requires unlocked Windows interactive desktop"]
    fn gui() {
        let p = Windows::default();
        let frame = p.capture().unwrap();
        aware().unwrap();
        let mut original = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
        assert_ne!(unsafe { GetCursorPos(&mut original) }, 0);
        for point in [
            Point { x: 0, y: 0 },
            Point {
                x: frame.width as i32 - 1,
                y: frame.height as i32 - 1,
            },
            Point {
                x: frame.width as i32 / 2,
                y: frame.height as i32 / 2,
            },
        ] {
            p.perform(Operation::Move(point)).unwrap();
            sleep(std::time::Duration::from_millis(50));
            let mut actual = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
            assert_ne!(unsafe { GetCursorPos(&mut actual) }, 0);
            assert_eq!((actual.x, actual.y), (point.x, point.y));
        }
        p.perform(Operation::Move(Point {
            x: original.x,
            y: original.y,
        }))
        .unwrap();
    }
}
