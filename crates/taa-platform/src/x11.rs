//! Pure Rust X11 GetImage/XTest backend; no libxcb or Xlib linkage.
use crate::{
    input::{self, Input, EVENT_DELAY},
    pixels::{CursorImage, Frame, Point},
    Button, Key, Operation, Platform, ScrollDirection,
};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    thread::sleep,
};
use x11rb::{
    connection::Connection,
    protocol::{
        randr::ConnectionExt as _, xfixes::ConnectionExt as _, xproto::*, xtest::ConnectionExt as _,
    },
    rust_connection::RustConnection,
};
#[derive(Default)]
pub struct X11 {
    state: Mutex<Option<Desktop>>,
    size: AtomicU64,
}
struct Desktop {
    held: std::collections::HashSet<u8>,
    auto_shift: std::collections::HashSet<u32>,
    conn: RustConnection,
    screen: usize,
    rect: (i16, i16, u16, u16),
}
fn err(e: impl std::fmt::Display) -> String {
    format!("X11: {e}; check DISPLAY, Xauthority and that XTest is enabled")
}
impl X11 {
    pub fn connect(display: &str) -> Result<Self, String> {
        let (conn, screen) = x11rb::connect(Some(display)).map_err(err)?;
        let d = Desktop::new(conn, screen)?;
        let size = ((d.rect.2 as u64) << 32) | d.rect.3 as u64;
        Ok(Self {
            state: Mutex::new(Some(d)),
            size: AtomicU64::new(size),
        })
    }
    fn with<T>(&self, f: impl FnOnce(&mut Desktop) -> Result<T, String>) -> Result<T, String> {
        let mut lock = self.state.lock().map_err(err)?;
        if lock.is_none() {
            let (c, s) = x11rb::connect(None).map_err(err)?;
            *lock = Some(Desktop::new(c, s)?);
        }
        let d = lock.as_mut().unwrap();
        d.refresh();
        self.size.store(
            ((d.rect.2 as u64) << 32) | d.rect.3 as u64,
            Ordering::Relaxed,
        );
        f(d)
    }
}
impl Desktop {
    fn new(conn: RustConnection, screen: usize) -> Result<Self, String> {
        conn.xtest_get_version(2, 2)
            .map_err(err)?
            .reply()
            .map_err(err)?;
        let root = &conn.setup().roots[screen];
        let rect = (0, 0, root.width_in_pixels, root.height_in_pixels);
        if let Ok(cookie) = conn.xfixes_query_version(5, 0) {
            let _ = cookie.reply();
        }
        let mut d = Self {
            conn,
            screen,
            rect,
            held: Default::default(),
            auto_shift: Default::default(),
        };
        d.refresh();
        Ok(d)
    }
    fn root(&self) -> u32 {
        self.conn.setup().roots[self.screen].root
    }
    fn refresh(&mut self) {
        if let Ok(cookie) = self.conn.get_geometry(self.root()) {
            if let Ok(g) = cookie.reply() {
                self.rect = (0, 0, g.width, g.height);
            }
        }
        if let Ok(cookie) = self.conn.randr_get_monitors(self.root(), true) {
            if let Ok(m) = cookie.reply() {
                if let Some(m) = m.monitors.iter().find(|m| m.primary) {
                    self.rect = (m.x, m.y, m.width, m.height);
                }
            }
        }
    }
    fn event(&self, kind: u8, detail: u8, x: i16, y: i16) -> Result<(), String> {
        self.conn
            .xtest_fake_input(kind, detail, 0, self.root(), x, y, 0)
            .map_err(err)?
            .check()
            .map_err(err)?;
        self.conn.flush().map_err(err)
    }
    fn mapping(&self) -> Result<GetKeyboardMappingReply, String> {
        let s = self.conn.setup();
        self.conn
            .get_keyboard_mapping(s.min_keycode, s.max_keycode - s.min_keycode + 1)
            .map_err(err)?
            .reply()
            .map_err(err)
    }
    fn keycode(&self, k: Key) -> Result<Option<(u8, bool)>, String> {
        let map = self.mapping()?;
        let stride = map.keysyms_per_keycode as usize;
        for (index, row) in map.keysyms.chunks(stride).enumerate() {
            if row.first() == Some(&k.0) {
                return Ok(Some((self.conn.setup().min_keycode + index as u8, false)));
            }
            if row.get(1) == Some(&k.0) {
                return Ok(Some((self.conn.setup().min_keycode + index as u8, true)));
            }
        }
        Ok(None)
    }
    fn character(&mut self, k: Key) -> Result<(), String> {
        if self.keycode(k)?.is_some() {
            let down = self.key(k, true);
            down.and(self.key(k, false))
        } else {
            let code = self.conn.setup().max_keycode;
            let old = self
                .conn
                .get_keyboard_mapping(code, 1)
                .map_err(err)?
                .reply()
                .map_err(err)?;
            self.conn
                .change_keyboard_mapping(
                    1,
                    code,
                    old.keysyms_per_keycode,
                    &vec![k.0; old.keysyms_per_keycode as usize],
                )
                .map_err(err)?
                .check()
                .map_err(err)?;
            let r = (|| {
                self.event(KEY_PRESS_EVENT, code, 0, 0)?;
                self.event(KEY_RELEASE_EVENT, code, 0, 0)?;
                sleep(EVENT_DELAY);
                Ok(())
            })();
            let restore = self
                .conn
                .change_keyboard_mapping(1, code, old.keysyms_per_keycode, &old.keysyms)
                .map_err(err)?
                .check()
                .map_err(err);
            r.and(restore)
        }
    }
}
impl Platform for X11 {
    fn name(&self) -> &str {
        "x11"
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
        self.with(|d| Ok((d.rect.2 as u32, d.rect.3 as u32)))
    }
    fn capture(&self) -> Result<Frame, String> {
        self.with(|d| {
            let (x, y, w, h) = d.rect;
            let img = d
                .conn
                .get_image(ImageFormat::Z_PIXMAP, d.root(), x, y, w, h, u32::MAX)
                .map_err(err)?
                .reply()
                .map_err(err)?;
            let setup = d.conn.setup();
            let format = setup
                .pixmap_formats
                .iter()
                .find(|p| p.depth == img.depth)
                .ok_or("X11: unknown pixel format")?;
            let visual = setup.roots[d.screen]
                .allowed_depths
                .iter()
                .flat_map(|d| &d.visuals)
                .find(|v| v.visual_id == img.visual)
                .ok_or("X11: unknown visual")?;
            let bpp = format.bits_per_pixel as usize;
            let pad = format.scanline_pad as usize;
            let stride = (w as usize * bpp).div_ceil(pad) * pad / 8;
            let mut f = Frame::solid(w as u32, h as u32, [0, 0, 0, 255]);
            for yy in 0..h as usize {
                for xx in 0..w as usize {
                    let p = &img.data[yy * stride + xx * bpp / 8..][..bpp / 8];
                    let mut v = 0u32;
                    for (j, b) in p.iter().enumerate() {
                        let shift = if setup.image_byte_order == ImageOrder::LSB_FIRST {
                            j * 8
                        } else {
                            (p.len() - 1 - j) * 8
                        };
                        v |= (*b as u32) << shift;
                    }
                    let di = (yy * w as usize + xx) * 4;
                    for (k, mask) in [visual.red_mask, visual.green_mask, visual.blue_mask]
                        .iter()
                        .enumerate()
                    {
                        if *mask != 0 {
                            f.rgba[di + k] = (((v & mask) >> mask.trailing_zeros()) * 255
                                / (mask >> mask.trailing_zeros()))
                                as u8;
                        }
                    }
                }
            }
            if let Ok(c) = d.conn.xfixes_get_cursor_image() {
                if let Ok(c) = c.reply() {
                    let mut rgba = Vec::with_capacity(c.cursor_image.len() * 4);
                    for p in c.cursor_image {
                        let a = p >> 24;
                        let unpremul =
                            |v: u32| (v * 255).checked_div(a).unwrap_or(0).min(255) as u8;
                        rgba.extend_from_slice(&[
                            unpremul((p >> 16) & 255),
                            unpremul((p >> 8) & 255),
                            unpremul(p & 255),
                            a as u8,
                        ]);
                    }
                    f.cursor = Some(CursorImage {
                        position: Point {
                            x: c.x as i32 - x as i32,
                            y: c.y as i32 - y as i32,
                        },
                        hotspot: Point {
                            x: c.xhot as i32,
                            y: c.yhot as i32,
                        },
                        width: c.width as u32,
                        height: c.height as u32,
                        rgba,
                    });
                }
            }
            if f.cursor.is_none() {
                if let Ok(c) = d.conn.query_pointer(d.root()).map_err(err)?.reply() {
                    f.cursor = Some(crate::pixels::arrow(Point {
                        x: c.root_x as i32 - x as i32,
                        y: c.root_y as i32 - y as i32,
                    }));
                }
            }
            Ok(f)
        })
    }
    fn perform(&self, op: Operation) -> Result<(), String> {
        self.with(|d| input::perform(d, op))
    }
}
impl Input for Desktop {
    fn motion(&mut self, p: Point) -> Result<(), String> {
        self.event(
            MOTION_NOTIFY_EVENT,
            0,
            (p.x + self.rect.0 as i32) as i16,
            (p.y + self.rect.1 as i32) as i16,
        )
    }
    fn button(&mut self, b: Button, down: bool, _: u8) -> Result<(), String> {
        self.event(
            if down {
                BUTTON_PRESS_EVENT
            } else {
                BUTTON_RELEASE_EVENT
            },
            match b {
                Button::Left => 1,
                Button::Middle => 2,
                Button::Right => 3,
            },
            0,
            0,
        )
    }
    fn key(&mut self, k: Key, down: bool) -> Result<(), String> {
        let (code, shift) = self
            .keycode(k)?
            .ok_or_else(|| format!("X11: key {:#x} unavailable in current keymap", k.0))?;
        let shift_code = self.keycode(Key(0xffe1))?.map(|p| p.0);
        if down
            && shift
            && self.auto_shift.is_empty()
            && shift_code.is_some_and(|s| !self.held.contains(&s))
        {
            self.event(KEY_PRESS_EVENT, shift_code.unwrap(), 0, 0)?;
            self.auto_shift.insert(k.0);
        }
        let result = self.event(
            if down {
                KEY_PRESS_EVENT
            } else {
                KEY_RELEASE_EVENT
            },
            code,
            0,
            0,
        );
        if down {
            self.held.insert(code);
        } else {
            self.held.remove(&code);
        }
        if !down && self.auto_shift.remove(&k.0) {
            result.and(self.event(KEY_RELEASE_EVENT, shift_code.unwrap(), 0, 0))
        } else {
            result
        }
    }

    fn scroll(&mut self, d: ScrollDirection, t: i64) -> Result<(), String> {
        if t == 0 {
            return Ok(());
        }
        let d = if t < 0 {
            match d {
                ScrollDirection::Up => ScrollDirection::Down,
                ScrollDirection::Down => ScrollDirection::Up,
                ScrollDirection::Left => ScrollDirection::Right,
                ScrollDirection::Right => ScrollDirection::Left,
            }
        } else {
            d
        };
        let b = match d {
            ScrollDirection::Up => 4,
            ScrollDirection::Down => 5,
            ScrollDirection::Left => 6,
            ScrollDirection::Right => 7,
        };
        for _ in 0..crate::notches(t.saturating_abs()) {
            self.event(BUTTON_PRESS_EVENT, b, 0, 0)?;
            self.event(BUTTON_RELEASE_EVENT, b, 0, 0)?;
        }
        Ok(())
    }
    fn text(&mut self, text: &str) -> Result<(), String> {
        for c in text.chars() {
            let k = match c {
                '\n' => Key(0xff0d),
                '\t' => Key(0xff09),
                _ => Key::character(c),
            };
            self.character(k)?;
            sleep(EVENT_DELAY);
        }
        Ok(())
    }
}
