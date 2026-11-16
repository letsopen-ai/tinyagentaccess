#![cfg(target_os = "linux")]
//! Run: cargo test -p taa-tools --test x11_e2e -- --ignored --nocapture
use base64::Engine;
use serde_json::json;
use std::{
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use taa_platform::{
    pixels::{Mapping, Point},
    x11::X11,
    Platform,
};
use taa_tools::{config::Config, execute};
use x11rb::{
    connection::Connection,
    protocol::{xproto::*, Event},
};
struct Xvfb(Child);
impl Drop for Xvfb {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[tokio::test]
#[ignore = "requires Xvfb binary; starts its own isolated 1920x1080 display"]
async fn real_x11_desktop() {
    let mut server = None;
    for n in 120..220 {
        if std::path::Path::new(&format!("/tmp/.X11-unix/X{n}")).exists() {
            continue;
        }
        let display = format!(":{n}");
        let child = Command::new("Xvfb")
            .args([&display, "-screen", "0", "1920x1080x24", "-nolisten", "tcp"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("install Xvfb");
        let mut guard = Xvfb(child);
        for _ in 0..50 {
            if let Ok((c, s)) = x11rb::connect(Some(&display)) {
                server = Some((guard, display, c, s));
                break;
            }
            if guard.0.try_wait().unwrap().is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        if server.is_some() {
            break;
        }
    }
    let (_server, display, c, s) = server.expect("free Xvfb display");
    let root = &c.setup().roots[s];
    let window = c.generate_id().unwrap();
    c.create_window(
        root.root_depth,
        window,
        root.root,
        300,
        180,
        1000,
        700,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new()
            .background_pixel(0xff0000)
            .event_mask(
                EventMask::BUTTON_PRESS
                    | EventMask::BUTTON_RELEASE
                    | EventMask::POINTER_MOTION
                    | EventMask::KEY_PRESS
                    | EventMask::KEY_RELEASE,
            ),
    )
    .unwrap()
    .check()
    .unwrap();
    c.map_window(window).unwrap().check().unwrap();
    c.set_input_focus(InputFocus::PARENT, window, x11rb::CURRENT_TIME)
        .unwrap()
        .check()
        .unwrap();
    c.flush().unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let ev = events.clone();
    let st = stop.clone();
    let recorder = thread::spawn(move || {
        while !st.load(Ordering::Relaxed) {
            while let Some(event) = c.poll_for_event().unwrap() {
                let sym = if let Event::KeyPress(k) = &event {
                    c.get_keyboard_mapping(k.detail, 1)
                        .unwrap()
                        .reply()
                        .unwrap()
                        .keysyms
                        .first()
                        .copied()
                } else {
                    None
                };
                ev.lock().unwrap().push((event, sym));
            }
            thread::sleep(Duration::from_millis(1));
        }
    });
    let p: Arc<dyn Platform> = Arc::new(X11::connect(&display).unwrap());
    let config = Config::default();
    let shot = execute("screenshot", &json!({}), &config, p.clone()).await;
    assert_eq!(shot["isError"], false, "{shot}");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(shot["content"][0]["data"].as_str().unwrap())
        .unwrap();
    let f = taa_platform::pixels::decode_png(&bytes).unwrap();
    assert_eq!((f.width, f.height), (1280, 720));
    let sample = ((200 * 1280 + 300) * 4) as usize;
    assert_eq!(&f.rgba[sample..sample + 3], &[255, 0, 0]);
    for (name, args) in [
        ("left_click", json!({"x":400,"y":300})),
        ("type_text", json!({"text":"a中😀é\n\t"})),
        ("key", json!({"keys":"ctrl+r"})),
        (
            "scroll",
            json!({"x":400,"y":300,"scroll_direction":"Up","scroll_amount":240}),
        ),
        (
            "scroll",
            json!({"x":400,"y":300,"scroll_direction":"Down","scroll_amount":120}),
        ),
        (
            "left_click_drag",
            json!({"start_x":400,"start_y":300,"end_x":500,"end_y":350}),
        ),
    ] {
        let r = execute(name, &args, &config, p.clone()).await;
        assert_eq!(r["isError"], false, "{name}: {r}");
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if events
            .lock()
            .unwrap()
            .iter()
            .filter(|(e, _)| matches!(e,Event::ButtonRelease(b) if b.detail==1))
            .count()
            >= 2
        {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    stop.store(true, Ordering::Relaxed);
    recorder.join().unwrap();
    let events = events.lock().unwrap();
    let expected = Mapping::new((1920, 1080), false).inverse(Point { x: 400, y: 300 });
    assert!(events.iter().any(|(e,_)|matches!(e,Event::ButtonPress(b) if b.detail==1&&b.root_x==expected.x as i16&&b.root_y==expected.y as i16)));
    for sym in [
        0x61, 0x01004e2d, 0x0101f600, 0x01000301, 0xff0d, 0xff09, 0xffe3, 0x72,
    ] {
        assert!(
            events.iter().any(|(_, s)| *s == Some(sym)),
            "missing keysym {sym:x}: {events:?}"
        );
    }
    assert!(events
        .iter()
        .any(|(e, _)| matches!(e,Event::KeyPress(k) if k.state.contains(KeyButMask::CONTROL))));
    for (button, count) in [(4, 2), (5, 1)] {
        assert_eq!(
            events
                .iter()
                .filter(|(e, _)| matches!(e,Event::ButtonPress(b) if b.detail==button))
                .count(),
            count
        );
    }
    assert!(events
        .iter()
        .any(|(e, _)| matches!(e,Event::MotionNotify(m) if m.state.contains(KeyButMask::BUTTON1))));
    let end = Mapping::new((1920, 1080), false).inverse(Point { x: 500, y: 350 });
    assert!(events.iter().any(|(e,_)|matches!(e,Event::ButtonRelease(b) if b.detail==1&&b.root_x==end.x as i16&&b.root_y==end.y as i16)));
}
