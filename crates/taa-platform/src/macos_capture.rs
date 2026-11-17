//! Capture policy shared with host tests; native calls are compiled only on macOS.
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub(crate) const CAPTURE_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) fn backend_name(screenshot_class_present: bool) -> &'static str {
    if screenshot_class_present {
        "macos-screencapturekit"
    } else {
        "macos-coregraphics"
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Strategy {
    ScreenCaptureKit,
    Legacy,
    Unavailable,
}

fn strategy(class_present: bool, legacy_symbol_present: bool) -> Strategy {
    match (class_present, legacy_symbol_present) {
        (true, _) => Strategy::ScreenCaptureKit,
        (false, true) => Strategy::Legacy,
        (false, false) => Strategy::Unavailable,
    }
}

// SCStreamErrorUserDeclined is -3801 in SCStreamError.h.
fn is_permission_error(preflight: bool, domain_is_sck: bool, code: isize) -> bool {
    !preflight || (domain_is_sck && code == -3801)
}

pub(crate) fn permission(screen: bool) -> String {
    let name = if screen {
        "Screen Recording"
    } else {
        "Accessibility"
    };
    format!("{name} permission is required: System Settings > Privacy & Security > {name} > enable <the terminal/app running tinyagentaccess>, then restart it")
}

// The callback owns a clone. Closing the slot atomically discards late results,
// including their retained images, without leaving a dangling stack pointer.
struct Completion<T> {
    result: Option<T>,
    closed: bool,
}
impl<T> Default for Completion<T> {
    fn default() -> Self {
        Self {
            result: None,
            closed: false,
        }
    }
}
impl<T> Completion<T> {
    fn finish(&mut self, result: T) {
        if !self.closed && self.result.is_none() {
            self.result = Some(result);
        }
    }
    fn take_or_close(&mut self, expired: bool) -> Option<T> {
        let result = self.result.take();
        if result.is_some() || expired {
            self.closed = true;
        }
        result
    }
}

fn wait<T>(
    slot: &Mutex<Completion<T>>,
    deadline: Instant,
    mut pump: impl FnMut(Duration),
) -> Result<T, String> {
    loop {
        let now = Instant::now();
        let expired = now >= deadline;
        if let Some(result) = slot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take_or_close(expired)
        {
            return Ok(result);
        }
        if expired {
            return Err("ScreenCaptureKit capture timed out after 10 seconds; check Screen Recording permission and an unlocked desktop".into());
        }
        pump((deadline - now).min(Duration::from_millis(10)));
    }
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use block2::RcBlock;
    use objc2::{runtime::AnyClass, AnyThread};
    use objc2_core_graphics::CGImage;
    use objc2_foundation::{NSArray, NSError};
    use objc2_screen_capture_kit::{
        SCContentFilter, SCScreenshotManager, SCShareableContent, SCStreamConfiguration,
        SCStreamErrorDomain,
    };
    use std::ffi::{c_char, c_void};

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFRunLoopDefaultMode: *const c_void;
        fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source: bool) -> i32;
    }
    unsafe extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGImageRetain(image: *mut c_void) -> *mut c_void;
        fn CGImageRelease(image: *mut c_void);
        fn CGPreflightScreenCaptureAccess() -> bool;
    }

    /// A +1 immutable CGImage. CoreGraphics images may be transferred between
    /// threads; the callback retains before returning and Drop balances it.
    pub(crate) struct Image(pub(crate) *mut c_void);
    unsafe impl Send for Image {}
    impl Drop for Image {
        fn drop(&mut self) {
            // SAFETY: constructed only from a non-null, owned CGImage reference.
            unsafe { CGImageRelease(self.0) }
        }
    }

    pub(crate) fn available() -> bool {
        AnyClass::get(c"SCScreenshotManager").is_some()
    }

    fn failure(stage: &str, error: Option<&NSError>) -> String {
        // SAFETY: preflight only queries authorization; it never opens a prompt.
        let (domain_is_sck, code) = error.map_or((false, 0), |e| {
            // SAFETY: this constant is available at the linked macOS 12.3 floor.
            (&*e.domain() == unsafe { SCStreamErrorDomain }, e.code())
        });
        if is_permission_error(
            unsafe { CGPreflightScreenCaptureAccess() },
            domain_is_sck,
            code,
        ) {
            return permission(true);
        }
        match error {
            Some(e) => format!("ScreenCaptureKit {stage} failed: {}", e.localizedDescription()),
            None => format!("ScreenCaptureKit {stage} returned no image/content; check Screen Recording permission and an unlocked desktop"),
        }
    }

    pub(crate) fn capture(display: u32, size: (u32, u32)) -> Result<Image, String> {
        let class_present = available();
        // Resolve the legacy API only when the screenshot class is absent.
        let symbol = if class_present {
            std::ptr::null_mut()
        } else {
            // SAFETY: RTLD_DEFAULT is -2 on Darwin.
            unsafe { dlsym((-2isize) as *mut c_void, c"CGDisplayCreateImage".as_ptr()) }
        };
        match strategy(class_present, !symbol.is_null()) {
            Strategy::Unavailable => {
                return Err("ScreenCaptureKit requires macOS 14+ and the legacy CGDisplayCreateImage API is unavailable".into());
            }
            Strategy::ScreenCaptureKit => {}
            Strategy::Legacy => {
                // SAFETY: strategy checked the symbol is non-null; the signature
                // matches CGDisplayCreateImage's C ABI.
                let legacy: unsafe extern "C" fn(u32) -> *mut c_void =
                    unsafe { std::mem::transmute(symbol) };
                let image = unsafe { legacy(display) };
                return if image.is_null() {
                    Err("CoreGraphics capture failed; check Screen Recording permission and an unlocked desktop".into())
                } else {
                    Ok(Image(image))
                };
            }
        }

        // A single deadline covers enumeration AND screenshot completion.
        let deadline = Instant::now() + CAPTURE_TIMEOUT;
        let slot = Arc::new(Mutex::new(Completion::default()));
        let completed = Arc::clone(&slot);
        let content_callback = RcBlock::new(
            move |content: *mut SCShareableContent, error: *mut NSError| {
                if completed.lock().unwrap_or_else(|e| e.into_inner()).closed {
                    return;
                }
                // SAFETY: callback pointers are valid borrows for this invocation.
                let content = unsafe { content.as_ref() };
                let error = unsafe { error.as_ref() };
                let Some(content) = content.filter(|_| error.is_none()) else {
                    completed
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .finish(Err(failure("display enumeration", error)));
                    return;
                };
                // SAFETY: content is a valid SCShareableContent from the framework.
                let displays = unsafe { content.displays() };
                let Some(display) = displays.iter().find(|d| {
                    // SAFETY: each array element is a retained SCDisplay.
                    unsafe { d.displayID() == display }
                }) else {
                    completed.lock().unwrap_or_else(|e| e.into_inner()).finish(Err("ScreenCaptureKit: primary display is missing from shareable displays; check Screen Recording permission and retry after display changes settle".into()));
                    return;
                };
                // SAFETY: the selected display and empty excluded-window array are
                // valid. This initializer includes desktop, dock and menu bar.
                let filter = unsafe {
                    SCContentFilter::initWithDisplay_excludingWindows(
                        SCContentFilter::alloc(),
                        &display,
                        &NSArray::new(),
                    )
                };
                // SAFETY: configuration is available at the linked 12.3 floor;
                // width/height are nonzero physical pixel dimensions from CG.
                let config = unsafe { SCStreamConfiguration::new() };
                unsafe {
                    config.setWidth(size.0 as usize);
                    config.setHeight(size.1 as usize);
                    config.setShowsCursor(false);
                }
                let image_completed = Arc::clone(&completed);
                // Defensively keep inputs alive in the copied completion block, including
                // after timeout. These SCK objects support use on callback queues.
                let keep_alive = (filter.clone(), config.clone());
                let image_callback =
                    RcBlock::new(move |image: *mut CGImage, error: *mut NSError| {
                        let _ = &keep_alive;
                        // SAFETY: callback error is borrowed for this invocation.
                        let error = unsafe { error.as_ref() };
                        let result = if error.is_some() || image.is_null() {
                            Err(failure("screenshot", error))
                        } else {
                            // SAFETY: non-null callback image is borrowed; retain it
                            // before handing ownership to the waiting thread.
                            Ok(Image(unsafe { CGImageRetain(image.cast()) }))
                        };
                        image_completed
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .finish(result);
                    });
                // SAFETY: the class was checked; valid retained inputs are kept
                // in the completion block, which the asynchronous API copies.
                unsafe {
                    SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
                        &filter,
                        &config,
                        Some(&image_callback),
                    )
                };
            },
        );
        // SAFETY: the API copies its block; Arc state survives a late callback.
        unsafe {
            SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
                false, true, &content_callback,
            );
        }
        wait(&slot, deadline, |duration| {
            // SAFETY: default mode is a framework-owned CFString; run only this
            // waiting thread's loop defensively, with no NSApplication. SCK
            // callbacks typically arrive on a dispatch queue; this is a bounded
            // 10 ms poll, not a dependency on run-loop delivery.
            let started = Instant::now();
            unsafe {
                CFRunLoopRunInMode(kCFRunLoopDefaultMode, duration.as_secs_f64(), true);
            }
            // A thread without run-loop sources can return immediately. Avoid
            // spinning while still pumping and observing completion every 10 ms.
            std::thread::sleep(duration.saturating_sub(started.elapsed()));
        })?
    }
}
#[cfg(target_os = "macos")]
pub(crate) use native::{available, capture};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_and_permissions() {
        assert_eq!(backend_name(true), "macos-screencapturekit");
        assert_eq!(backend_name(false), "macos-coregraphics");
        assert!(
            permission(true).contains("System Settings > Privacy & Security > Screen Recording")
        );
        assert!(permission(false).contains("Accessibility"));
        assert_eq!(CAPTURE_TIMEOUT, Duration::from_secs(10));
    }

    #[test]
    fn fallback_strategy() {
        assert_eq!(strategy(true, true), Strategy::ScreenCaptureKit);
        assert_eq!(strategy(true, false), Strategy::ScreenCaptureKit);
        assert_eq!(strategy(false, true), Strategy::Legacy);
        assert_eq!(strategy(false, false), Strategy::Unavailable);
    }

    #[test]
    fn permission_mapping() {
        for domain in [false, true] {
            for code in [-3801, -3802, 0] {
                assert!(is_permission_error(false, domain, code));
                assert_eq!(
                    is_permission_error(true, domain, code),
                    domain && code == -3801
                );
            }
        }
    }

    #[test]
    fn late_results_release_ownership() {
        let value = Arc::new(());
        let observer = Arc::downgrade(&value);
        let slot = Mutex::new(Completion::<Arc<()>>::default());
        assert!(wait(&slot, Instant::now(), |_| {}).is_err());
        slot.lock().unwrap().finish(value);
        assert!(observer.upgrade().is_none());
    }

    #[test]
    fn completion_and_late_callback() {
        let slot = Arc::new(Mutex::new(Completion::default()));
        let callback = Arc::clone(&slot);
        let mut count = 0;
        assert_eq!(
            wait(&slot, Instant::now() + CAPTURE_TIMEOUT, |_| {
                count += 1;
                callback.lock().unwrap().finish(42);
            }),
            Ok(42)
        );
        assert_eq!(count, 1);
        let slot = Mutex::new(Completion::<i32>::default());
        assert!(wait(&slot, Instant::now(), |_| panic!(
            "expired waits must not pump"
        ))
        .unwrap_err()
        .contains("timed out"));
        slot.lock().unwrap().finish(7);
        assert!(slot.lock().unwrap().result.is_none());
    }
}
