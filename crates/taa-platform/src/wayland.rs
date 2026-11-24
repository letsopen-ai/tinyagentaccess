//! Portal capture avoids libpipewire; the dedicated runtime stays alive between calls.
use crate::{
    input::{self, Input, EVENT_DELAY},
    pixels::{Frame, Point},
    Button, Key, Operation, Platform, ScrollDirection,
};
use ashpd::desktop::{
    remote_desktop::{Axis, DeviceType, KeyState, RemoteDesktop},
    screencast::Screencast,
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
}
fn error(e: impl std::fmt::Display) -> String {
    format!("xdg-desktop-portal: {e}; install xdg-desktop-portal and the GNOME/KDE portal backend, and approve Screenshot/RemoteDesktop screen and input sharing")
}
const CONSENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
async fn bounded<T>(
    future: impl std::future::Future<Output = Result<T, ashpd::Error>>,
) -> Result<T, ashpd::Error> {
    bounded_for(CONSENT_TIMEOUT, future).await
}
async fn bounded_for<T>(
    limit: std::time::Duration,
    future: impl std::future::Future<Output = Result<T, ashpd::Error>>,
) -> Result<T, ashpd::Error> {
    tokio::time::timeout(limit, future).await.map_err(|_| {
        ashpd::Error::ParseError(
            "portal approval timed out after 60 seconds; retry and approve sharing",
        )
    })?
}
// ashpd 0.12's typed SelectSources always sends persist_mode. Combined sessions
// must omit it, so use its public zbus proxy with the same request protocol.
async fn select_monitor(
    cast: &Screencast<'_>,
    session: &Session<'_, RemoteDesktop<'_>>,
) -> Result<(), ashpd::Error> {
    use ashpd::{
        zbus,
        zvariant::{OwnedObjectPath, OwnedValue, Value},
    };
    use std::collections::HashMap;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let token = format!("taa{}", NEXT.fetch_add(1, Ordering::Relaxed));
    let sender = cast
        .connection()
        .unique_name()
        .ok_or(ashpd::Error::ParseError("portal bus has no unique name"))?
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    let request = zbus::Proxy::new(
        cast.connection(),
        "org.freedesktop.portal.Desktop",
        path.as_str(),
        "org.freedesktop.portal.Request",
    )
    .await?;
    let mut responses = request.receive_signal("Response").await?;
    let options = HashMap::from([
        ("handle_token", Value::from(token.as_str())),
        ("types", Value::from(1u32)),
        ("multiple", Value::from(false)),
        ("cursor_mode", Value::from(1u32)),
    ]);
    let returned: OwnedObjectPath = cast.call("SelectSources", &(session, options)).await?;
    if returned.as_str() != path {
        return Err(ashpd::Error::ParseError("unexpected portal request path"));
    }
    let response = responses.next().await.ok_or(ashpd::Error::NoResponse)?;
    let (status, _): (u32, HashMap<String, OwnedValue>) = response.body().deserialize()?;
    if status != 0 {
        return Err(ashpd::Error::ParseError(
            "monitor selection denied or cancelled",
        ));
    }
    Ok(())
}
fn token_path() -> Result<PathBuf, String> {
    crate::state_dir()
        .map(|d| d.join("portal-restore-token"))
        .map_err(|e| format!("{e}; restore token disabled"))
}
fn save_token(token: &str) -> Result<(), String> {
    save_token_at(&token_path()?, token)
}
fn save_token_at(path: &std::path::Path, token: &str) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
    let dir = path.parent().ok_or("token path has no parent")?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(error)?;
    let meta = std::fs::symlink_metadata(dir).map_err(error)?;
    if !meta.is_dir() || meta.file_type().is_symlink() || meta.uid() != unsafe { libc::geteuid() } {
        return Err("unsafe token directory".into());
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(error)?;
    let temporary = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(error)?;
    use std::io::Write;
    let result = file
        .write_all(token.as_bytes())
        .and_then(|_| file.sync_all())
        .and_then(|_| std::fs::rename(&temporary, path));
    let _ = std::fs::remove_file(&temporary);
    result.map_err(error)
}
fn read_token() -> Option<String> {
    read_token_at(&token_path().ok()?)
}
fn read_token_at(path: &std::path::Path) -> Option<String> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let f = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let meta = f.metadata().ok()?;
    if meta.uid() != unsafe { libc::geteuid() }
        || !meta.is_file()
        || meta.mode() & 0o077 != 0
        || meta.len() > 65536
    {
        return None;
    }
    let mut t = String::new();
    use std::io::Read;
    f.take(65537).read_to_string(&mut t).ok()?;
    if t.len() > 65536 {
        return None;
    }
    Some(t.trim().to_owned()).filter(|t| !t.is_empty())
}
const MAX_SCREENSHOT_BYTES: u64 = 256 * 1024 * 1024;
fn read_screenshot(path: &std::path::Path) -> Result<Vec<u8>, String> {
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
    };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(error)?;
    let meta = file.metadata().map_err(error)?;
    if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } || meta.nlink() != 1 {
        return Err("portal screenshot must be an owned regular file with one link".into());
    }
    let result = (|| {
        if meta.len() > MAX_SCREENSHOT_BYTES {
            return Err("portal screenshot exceeds 256 MiB file limit".into());
        }
        let mut data = vec![];
        file.take(MAX_SCREENSHOT_BYTES + 1)
            .read_to_end(&mut data)
            .map_err(error)?;
        if data.len() as u64 > MAX_SCREENSHOT_BYTES {
            return Err("portal screenshot exceeds 256 MiB file limit".into());
        }
        Ok(data)
    })();
    // Do not unlink a replacement path or symlink.
    if std::fs::symlink_metadata(path)
        .is_ok_and(|now| now.dev() == meta.dev() && now.ino() == meta.ino())
    {
        let _ = std::fs::remove_file(path);
    }
    result
}
fn compatible_geometry(physical: (u32, u32), logical: (u32, u32)) -> Result<(), String> {
    let sx = physical.0 as f64 / logical.0 as f64;
    let sy = physical.1 as f64 / logical.1 as f64;
    if logical.0 == 0
        || logical.1 == 0
        || physical.0 == 0
        || physical.1 == 0
        || (sx - sy).abs() > 0.01 * sx.max(sy)
    {
        return Err("Wayland Screenshot/RemoteDesktop geometry mismatch; multi-monitor control is unsupported. Use a single monitor and retry".into());
    }
    Ok(())
}
impl Drop for Desktop {
    fn drop(&mut self) {
        blocking_cleanup(|| {
            close(self);
            if let Some(rt) = self.rt.take() {
                rt.shutdown_background();
            }
        });
    }
}
// Drop may run inside Tokio (including on CLI shutdown). Close the session and
// dispose of its runtime on a thread where nested block_on is safe.
fn blocking_cleanup(cleanup: impl FnOnce() + Send) {
    std::thread::scope(|scope| {
        scope
            .spawn(cleanup)
            .join()
            .expect("portal cleanup panicked");
    });
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
                let r = bounded(RemoteDesktop::new()).await?;
                let s = bounded(r.create_session()).await?;
                let outcome = bounded(async {
                    let token = read_token();
                    r.select_devices(
                        &s,
                        DeviceType::Keyboard | DeviceType::Pointer,
                        token.as_deref(),
                        PersistMode::ExplicitlyRevoked,
                    )
                    .await?
                    .response()?;
                    let cast = Screencast::new().await?;
                    select_monitor(&cast, &s).await?;
                    let selected = r.start(&s, None).await?.response()?;
                    if let Some(t) = selected.restore_token() {
                        if let Err(e) = save_token(t) {
                            eprintln!("portal token persistence unavailable: {e}");
                        }
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
                    Ok::<_, ashpd::Error>((node, (size.0 as u32, size.1 as u32)))
                })
                .await;
                match outcome {
                    Ok((node, size)) => Ok((r, s, node, size)),
                    Err(e) => {
                        let _ = bounded(s.close()).await;
                        Err(e)
                    }
                }
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
                .block_on(bounded(async {
                    let shot = ashpd::desktop::screenshot::Screenshot::request()
                        .interactive(false)
                        .send()
                        .await?
                        .response()?;
                    let path = shot.uri().to_file_path().map_err(|_| {
                        ashpd::Error::ParseError("Screenshot returned a non-local URI")
                    })?;
                    read_screenshot(&path).map_err(|_| {
                        ashpd::Error::ParseError(
                            "unsafe, oversized, or unreadable portal screenshot file",
                        )
                    })
                }))
                .map_err(error)?;
        let mut f = crate::pixels::decode_png(&bytes)?;
        self.physical = (f.width, f.height);
        f.logical_size = self.logical;
        // Screenshot portal cannot exclude its own cursor; avoid drawing a second stale arrow.
        Ok(f)
    }
}
impl Wayland {
    fn with<T>(&self, f: impl FnOnce(&mut Desktop) -> Result<T, String>) -> Result<T, String> {
        let mut lock = self.state.lock().unwrap_or_else(|e| e.into_inner());
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
        let f = self.capture()?;
        Ok((f.width, f.height))
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
            if matches!(
                op,
                Operation::Move(_)
                    | Operation::Click(..)
                    | Operation::Button(..)
                    | Operation::Drag(..)
                    | Operation::Scroll(..)
            ) {
                d.capture()?;
                if d.physical != self.screen_size() { return Err("Wayland capture dimensions changed during pointer operation; retry from a new screenshot".into()); }
                compatible_geometry(d.physical, d.logical)?;
            }
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
        let _ = d.runtime().block_on(bounded(s.close()));
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
        if k.0 == 0x11000000 {
            return Err("Wayland: unsupported special key fn".into());
        }
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
            let down = self.key(k, true);
            down.and(self.key(k, false))?;
            sleep(EVENT_DELAY);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn check_blocking_cleanup() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut completed = false;
        blocking_cleanup(|| {
            assert!(tokio::runtime::Handle::try_current().is_err());
            rt.block_on(async { completed = true });
            drop(rt);
        });
        assert!(completed, "cleanup must finish before returning");
    }
    #[test]
    fn blocking_cleanup_outside_runtime() {
        check_blocking_cleanup();
        drop(Desktop::new().unwrap());
    }
    #[tokio::test]
    async fn blocking_cleanup_inside_runtime() {
        check_blocking_cleanup();
        drop(Desktop::new().unwrap());
    }
    #[tokio::test]
    async fn portal_timeout_is_bounded() {
        let result = bounded_for(
            std::time::Duration::from_millis(10),
            std::future::pending::<Result<(), ashpd::Error>>(),
        )
        .await;
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }
    #[test]
    fn token_permissions_trimming_and_cleanup() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let dir = std::env::temp_dir().join(format!("taa-token-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("token");
        save_token_at(&path, "abc\n").unwrap();
        assert_eq!(read_token_at(&path).as_deref(), Some("abc"));
        assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_token_at(&path).is_none());
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(save_token_at(&path, "new").is_err());
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            1,
            "temporary file leaked"
        );
        std::fs::remove_dir(&path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn ratios_and_screenshot_file_safety() {
        assert!(compatible_geometry((3840, 2160), (1920, 1080)).is_ok());
        assert!(compatible_geometry((3840, 1080), (1920, 1080)).is_err());
        assert!(compatible_geometry((0, 1080), (1920, 1080)).is_err());
        let dir = std::env::temp_dir().join(format!("taa-file-safety-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("target");
        let link = dir.join("link");
        std::fs::write(&target, b"test").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(read_screenshot(&link).is_err());
        assert!(target.exists());
        let large = dir.join("large");
        let f = std::fs::File::create(&large).unwrap();
        f.set_len(MAX_SCREENSHOT_BYTES + 1).unwrap();
        assert!(read_screenshot(&large).is_err());
        assert!(!large.exists());
        assert_eq!(read_screenshot(&target).unwrap(), b"test");
        assert!(!target.exists());
        std::fs::remove_file(link).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
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
