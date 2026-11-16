use crate::{Button, Key, KeyCombo, Operation, Point, ScrollDirection};
use std::{thread::sleep, time::Duration};
pub const EVENT_DELAY: Duration = Duration::from_millis(5);
pub trait Input {
    fn motion(&mut self, p: Point) -> Result<(), String>;
    fn button(&mut self, b: Button, down: bool, count: u8) -> Result<(), String>;
    /// Native button events do not move the cursor. Backends requiring a location
    /// must use the current cursor position for this cleanup operation.
    fn release_left(&mut self) -> Result<(), String> {
        self.button(Button::Left, false, 1)
    }
    fn key(&mut self, k: Key, down: bool) -> Result<(), String>;
    fn scroll(&mut self, d: ScrollDirection, ticks: i64) -> Result<(), String>;
    fn text(&mut self, text: &str) -> Result<(), String>;
}
fn held(
    i: &mut dyn Input,
    keys: &KeyCombo,
    f: impl FnOnce(&mut dyn Input) -> Result<(), String>,
) -> Result<(), String> {
    let mut pressed = vec![];
    let mut result = Ok(());
    for k in keys {
        pressed.push(*k);
        if let Err(e) = i.key(*k, true) {
            result = Err(e);
            break;
        }
        sleep(EVENT_DELAY);
    }
    if result.is_ok() {
        result = f(i);
    }
    for k in pressed.iter().rev() {
        let r = i.key(*k, false);
        if result.is_ok() {
            result = r;
        }
    }
    result
}
pub fn perform(i: &mut dyn Input, op: Operation) -> Result<(), String> {
    match op {
        Operation::ReleaseLeft => i.release_left(),
        Operation::Move(p) => i.motion(p),
        Operation::Click(p, b, n, mods) => {
            i.motion(p)?;
            held(i, &mods, |i| {
                for count in 1..=n {
                    let down = i.button(b, true, count);
                    let r = down.and(i.button(b, false, count));
                    if count < n {
                        sleep(Duration::from_millis(60));
                    }
                    r?;
                }
                Ok(())
            })
        }
        Operation::Button(p, down, mods) => {
            i.motion(p)?;
            held(i, &mods, |i| i.button(Button::Left, down, 1))
        }
        Operation::Drag(a, b) => {
            i.motion(a)?;
            if let Err(e) = i.button(Button::Left, true, 1) {
                let _ = i.button(Button::Left, false, 1);
                return Err(e);
            }
            sleep(EVENT_DELAY);
            let moves = (|| {
                for step in 1..=12 {
                    i.motion(Point {
                        x: a.x + (b.x - a.x) * step / 12,
                        y: a.y + (b.y - a.y) * step / 12,
                    })?;
                    sleep(EVENT_DELAY);
                }
                Ok(())
            })();
            let release = i.button(Button::Left, false, 1);
            moves.and(release)
        }
        Operation::Scroll(p, d, t, mods) => {
            if t.unsigned_abs() > crate::MAX_SCROLL_TICKS as u64 {
                return Err("scroll amount exceeds safety limit".into());
            }
            i.motion(p)?;
            held(i, &mods, |i| i.scroll(d, t))
        }
        Operation::Key(keys, d) => held(i, &keys, |_| {
            sleep(d);
            Ok(())
        }),
        Operation::Text(t) => i.text(&t),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Fake {
        events: Vec<(Key, bool)>,
        fail: bool,
    }
    impl Input for Fake {
        fn motion(&mut self, _: Point) -> Result<(), String> {
            Ok(())
        }
        fn button(&mut self, _: Button, _: bool, _: u8) -> Result<(), String> {
            Err("injected button error".into())
        }
        fn key(&mut self, k: Key, down: bool) -> Result<(), String> {
            if self.fail && down && k == Key(2) {
                return Err("injected key error".into());
            }
            self.events.push((k, down));
            Ok(())
        }
        fn scroll(&mut self, _: ScrollDirection, _: i64) -> Result<(), String> {
            Ok(())
        }
        fn text(&mut self, _: &str) -> Result<(), String> {
            Ok(())
        }
    }
    #[test]
    fn release_in_reverse_even_on_failure() {
        let keys = vec![Key(1), Key(2), Key(3)];
        let mut f = Fake::default();
        perform(&mut f, Operation::Key(keys.clone(), Duration::ZERO)).unwrap();
        assert_eq!(
            f.events,
            vec![
                (Key(1), true),
                (Key(2), true),
                (Key(3), true),
                (Key(3), false),
                (Key(2), false),
                (Key(1), false)
            ]
        );
        let mut f = Fake {
            fail: true,
            ..Fake::default()
        };
        assert!(perform(&mut f, Operation::Key(keys.clone(), Duration::ZERO)).is_err());
        assert_eq!(
            f.events,
            vec![(Key(1), true), (Key(2), false), (Key(1), false)]
        );
        let mut f = Fake::default();
        assert!(perform(
            &mut f,
            Operation::Click(Point { x: 0, y: 0 }, Button::Left, 1, keys)
        )
        .is_err());
        assert_eq!(
            &f.events[3..],
            &[(Key(3), false), (Key(2), false), (Key(1), false)]
        );
    }
}
