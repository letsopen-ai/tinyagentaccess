//! Portal capture avoids libpipewire; the dedicated runtime stays alive between calls.
use crate::{
    input::{self, Input, EVENT_DELAY},
    pixels::{Frame, Point},
    Button, Key, Operation, Platform, ScrollDirection,
};
use ashpd::desktop::{
    remote_desktop::{Axis, DeviceType, KeyState, RemoteDesktop},
    screencast::{CursorMode, Screencast, SourceType},
    PersistMode, Session,
};
use futures_util::StreamExt;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread::sleep,
};
#[derive(Default)]
pub struct Wayland {
    state: Mutex<Option<Desktop>>,
    size: AtomicU64,
}
struct Desktop {
    rt: Option<tokio::runtime::Runtime>,
    session: Option<Arc<Session<'static, RemoteDesktop<'static>>>>,
    closed: Arc<AtomicBool>,
    watch: Option<tokio::task::JoinHandle<()>>,
    remote: Option<RemoteDesktop<'static>>,
    stream: u32,
    logical: (u32, u32),
    physical: (u32, u32),
    cursor: Option<Point>,
}
fn error(e: impl std::fmt::Display) -> String {
    format!("xdg-desktop-portal: {e}; install xdg-desktop-portal and the GNOME/KDE portal backend, and approve Screenshot/RemoteDesktop screen and input sharing")
}
fn token_path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
        })
        .join("tinyagentaccess/portal-restore-token")
}
fn save_token(token: &str) -> Result<(), String> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let path = token_path();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(error)?;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(error)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(error)?;
    use std::io::Write;
    let result = file
        .write_all(token.as_bytes())
        .and_then(|_| std::fs::rename(&temporary, &path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(error)
}
impl Drop for Desktop {
    fn drop(&mut self) {
        if let Some(rt) = self.rt.take() {
            rt.shutdown_background();
        }
    }
}
impl Desktop {
    fn runtime(&self) -> &tokio::runtime::Runtime {
        self.rt.as_ref().unwrap()
    }
    fn new() -> Result<Self, String> {
        Ok(Self {
            rt: Some(
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(1)
                    .enable_all()
                    .build()
                    .map_err(error)?,
            ),
            session: None,
            closed: Arc::new(AtomicBool::new(false)),
            watch: None,
            remote: None,
            stream: 0,
            logical: (1280, 720),
            physical: (1280, 720),
            cursor: None,
        })
    }
    fn session(&mut self) -> Result<(), String> {
        if self.closed.load(Ordering::Relaxed) {
            close(self);
        }
        if self.session.is_some() {
            return Ok(());
        }
        let (remote, session, stream, logical) = self
            .runtime()
            .block_on(async {
                let r = RemoteDesktop::new().await?;
                let s = r.create_session().await?;
                let token = std::fs::read_to_string(token_path()).ok();
                r.select_devices(
                    &s,
                    DeviceType::Keyboard | DeviceType::Pointer,
                    token.as_deref(),
                    PersistMode::ExplicitlyRevoked,
                )
                .await?
                .response()?;
                let cast = Screencast::new().await?;
                cast.select_sources(
                    &s,
                    CursorMode::Hidden,
                    SourceType::Monitor.into(),
                    false,
                    None,
                    PersistMode::ExplicitlyRevoked,
                )
                .await?
                .response()?;
                let selected = r.start(&s, None).await?.response()?;
                if let Some(t) = selected.restore_token() {
                    save_token(t).map_err(|_| {
                        ashpd::Error::ParseError("could not save portal restore token")
                    })?;
                }
                let stream = selected.streams().and_then(|s| s.first()).ok_or_else(|| {
                    ashpd::Error::ParseError("RemoteDesktop returned no monitor stream")
                })?;
                let size = stream
                    .size()
                    .ok_or_else(|| ashpd::Error::ParseError("monitor stream has no size"))?;
                if size.0 <= 0 || size.1 <= 0 {
                    return Err(ashpd::Error::ParseError("invalid stream size"));
                }
                let node = stream.pipe_wire_node_id();
                Ok::<_, ashpd::Error>((r, s, node, (size.0 as u32, size.1 as u32)))
            })
            .map_err(error)?;
        self.remote = Some(remote);
        let session = Arc::new(session);
        let watch = session.clone();
        let closed = self.closed.clone();
        closed.store(false, Ordering::Relaxed);
        self.watch = Some(self.runtime().spawn(async move {
            if let Ok(mut events) = watch.receive_closed().await {
                let _ = events.next().await;
            }
            closed.store(true, Ordering::Relaxed);
        }));
        self.session = Some(session);
        self.stream = stream;
        self.logical = logical;
        Ok(())
    }
    fn capture(&mut self) -> Result<Frame, String> {
        let bytes =
            self.runtime()
                .block_on(async {
                    let shot = ashpd::desktop::screenshot::Screenshot::request()
                        .interactive(false)
                        .send()
                        .await?
                        .response()?;
                    let path = shot.uri().to_file_path().map_err(|_| {
                        ashpd::Error::ParseError("Screenshot returned a non-local URI")
                    })?;
                    let data = std::fs::read(&path);
                    let _ = std::fs::remove_file(path);
                    data.map_err(|_| ashpd::Error::ParseError("could not read screenshot file"))
                })
                .map_err(error)?;
        let mut f = crate::pixels::decode_png(&bytes)?;
        self.physical = (f.width, f.height);
        f.logical_size = self.logical;
        f.cursor = self.cursor.map(crate::pixels::arrow);
        Ok(f)
    }
}
impl Wayland {
    fn with<T>(&self, f: impl FnOnce(&mut Desktop) -> Result<T, String>) -> Result<T, String> {
        let mut lock = self.state.lock().map_err(error)?;
        if lock.is_none() {
            *lock = Some(Desktop::new()?);
        }
        f(lock.as_mut().unwrap())
    }
}
impl Platform for Wayland {
    fn name(&self) -> &str {
        "wayland-portal"
    }
    fn screen_size(&self) -> (u32, u32) {
        let s = self.size.load(Ordering::Relaxed);
        if s == 0 {
            (0, 0)
        } else {
            ((s >> 32) as u32, s as u32)
        }
    }
    fn capture_geometry(&self) -> Result<(u32, u32), String> {
        if self.size.load(Ordering::Relaxed) == 0 {
            let f = self.capture()?;
            Ok((f.width, f.height))
        } else {
            Ok(self.screen_size())
        }
    }
    fn capture(&self) -> Result<Frame, String> {
        let f = self.with(|d| d.capture())?;
        self.size.store(
            ((f.width as u64) << 32) | f.height as u64,
            Ordering::Relaxed,
        );
        Ok(f)
    }
    fn perform(&self, op: Operation) -> Result<(), String> {
        self.with(|d| {
            d.session()?;
            let result = input::perform(d, op);
            if result.is_err() {
                self::close(d);
            }
            result
        })
    }
}
fn close(d: &mut Desktop) {
    if let Some(w) = d.watch.take() {
        w.abort();
    }
    if let Some(s) = d.session.take() {
        let _ = d.runtime().block_on(s.close());
    }
    d.remote = None;
}
fn state(down: bool) -> KeyState {
    if down {
        KeyState::Pressed
    } else {
        KeyState::Released
    }
}
impl Input for Desktop {
    fn motion(&mut self, p: Point) -> Result<(), String> {
        let (x, y) = crate::pixels::logical_point(p, self.physical, self.logical);
        self.runtime()
            .block_on(
                self.remote
                    .as_ref()
                    .unwrap()
                    .notify_pointer_motion_absolute(
                        self.session.as_ref().unwrap(),
                        self.stream,
                        x,
                        y,
                    ),
            )
            .map_err(error)?;
        self.cursor = Some(p);
        Ok(())
    }
    fn button(&mut self, b: Button, down: bool, _: u8) -> Result<(), String> {
        self.runtime()
            .block_on(self.remote.as_ref().unwrap().notify_pointer_button(
                self.session.as_ref().unwrap(),
                match b {
                    Button::Left => 0x110,
                    Button::Right => 0x111,
                    Button::Middle => 0x112,
                },
                state(down),
            ))
            .map_err(error)
    }
    fn key(&mut self, k: Key, down: bool) -> Result<(), String> {
        self.runtime()
            .block_on(self.remote.as_ref().unwrap().notify_keyboard_keysym(
                self.session.as_ref().unwrap(),
                k.0 as i32,
                state(down),
            ))
            .map_err(error)
    }
    fn scroll(&mut self, d: ScrollDirection, t: i64) -> Result<(), String> {
        if t == 0 {
            return Ok(());
        }
        let sign = if matches!(d, ScrollDirection::Up | ScrollDirection::Left) {
            -1
        } else {
            1
        } * if t < 0 { -1 } else { 1 };
        self.runtime()
            .block_on(self.remote.as_ref().unwrap().notify_pointer_axis_discrete(
                self.session.as_ref().unwrap(),
                if matches!(d, ScrollDirection::Up | ScrollDirection::Down) {
                    Axis::Vertical
                } else {
                    Axis::Horizontal
                },
                (crate::notches(t.saturating_abs()).min(i32::MAX as u32) as i32) * sign,
            ))
            .map_err(error)
    }
    fn text(&mut self, t: &str) -> Result<(), String> {
        for c in t.chars() {
            let k = match c {
                '\n' => Key(0xff0d),
                '\t' => Key(0xff09),
                _ => Key::character(c),
            };
            self.key(k, true)?;
            self.key(k, false)?;
            sleep(EVENT_DELAY);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guidance() {
        let e = error("denied");
        assert!(e.contains("xdg-desktop-portal"));
        assert!(e.contains("approve"));
    }
    #[test]
    #[ignore = "requires single-monitor GNOME/KDE Wayland desktop with portal consent; see docs/testing.md"]
    fn gui() {
        let p = Wayland::default();
        let f = p.capture().unwrap();
        f.validate().unwrap();
    }
}
