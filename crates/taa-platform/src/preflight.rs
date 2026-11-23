//! No-prompt diagnostics for `tinyagentaccess doctor`.
//!
//! Everything here is read-only: it never requests permission, captures a
//! frame, posts input, opens a display connection, or reads the TCC database.

/// macOS permission preflight results (no-prompt queries only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MacosPreflight {
    pub screen_recording: bool,
    pub accessibility: bool,
    pub screencapturekit_available: bool,
    pub backend: &'static str,
}

/// Returns `None` on every OS except macOS; never fabricates results.
pub fn macos() -> Option<MacosPreflight> {
    #[cfg(target_os = "macos")]
    {
        let screencapturekit_available = crate::macos_capture::available();
        Some(MacosPreflight {
            screen_recording: crate::macos::preflight_screen_recording(),
            accessibility: crate::macos::preflight_accessibility(),
            screencapturekit_available,
            backend: crate::macos_capture::backend_name(screencapturekit_available),
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Same rules as `detect`: wayland if `WAYLAND_DISPLAY` is set or
/// `XDG_SESSION_TYPE=wayland`; else x11 if `DISPLAY` is set; else unavailable.
pub fn linux_session(
    wayland_display: bool,
    xdg_session_wayland: bool,
    display: bool,
) -> &'static str {
    if wayland_display || xdg_session_wayland {
        "wayland"
    } else if display {
        "x11"
    } else {
        "unavailable"
    }
}

/// Classifies the current process environment without connecting to anything.
pub fn linux_session_from_env() -> &'static str {
    linux_session(
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
        std::env::var("XDG_SESSION_TYPE").is_ok_and(|s| s == "wayland"),
        std::env::var_os("DISPLAY").is_some(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_classification_matches_detect() {
        assert_eq!(linux_session(true, false, true), "wayland");
        assert_eq!(linux_session(false, true, false), "wayland");
        assert_eq!(linux_session(false, false, true), "x11");
        assert_eq!(linux_session(false, false, false), "unavailable");
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn no_fake_macos_results() {
        assert!(macos().is_none());
    }

    /// The doctor preflight path (this file plus the macOS wrappers it calls)
    /// must never reach request/prompt APIs or the TCC database.
    #[test]
    fn preflight_path_has_no_prompt_apis() {
        let own = include_str!("preflight.rs");
        let own = own.split("#[cfg(test)]").next().unwrap();
        let mac = include_str!("macos.rs");
        let begin = mac.find("// doctor-preflight-begin").unwrap();
        let end = mac.find("// doctor-preflight-end").unwrap();
        let wrappers = &mac[begin..end];
        assert!(wrappers.contains("CGPreflightScreenCaptureAccess"));
        assert!(wrappers.contains("AXIsProcessTrusted()"));
        // FFI bindings the wrappers call, and the class probe (not `capture`).
        let ffi_mac = &mac[..begin];
        let ffi_a = ffi_mac.find("fn CGPreflightScreenCaptureAccess").unwrap();
        let ffi_b = ffi_mac.find("fn AXIsProcessTrusted").unwrap();
        let ffi_a = &ffi_mac[ffi_a..ffi_a + "fn CGPreflightScreenCaptureAccess() -> bool;".len()];
        let ffi_b = &ffi_mac[ffi_b..ffi_b + "fn AXIsProcessTrusted() -> bool;".len()];
        assert_eq!(ffi_a, "fn CGPreflightScreenCaptureAccess() -> bool;");
        assert_eq!(ffi_b, "fn AXIsProcessTrusted() -> bool;");
        let cap = include_str!("macos_capture.rs");
        let pb = cap.find("pub(crate) fn available() -> bool {").unwrap();
        let probe = &cap[pb..pb + cap[pb..].find("\n    }\n").unwrap()];
        assert!(probe.contains("AnyClass::get"));
        for text in [own, wrappers, ffi_a, ffi_b, probe] {
            for banned in FORBIDDEN {
                assert!(!text.contains(banned), "forbidden `{banned}`");
            }
        }
    }

    const FORBIDDEN: &[&str] = &[
        "CGRequestScreenCaptureAccess",
        "CGRequestPostEventAccess",
        "AXIsProcessTrustedWithOptions",
        "kAXTrustedCheckOptionPrompt",
        "TCC.db",
        "NSWorkspace",
        "x-apple.systempreferences",
        "CGEventPost",
        "SCShareableContent",
        "SCStream",
        "capture(",
    ];
}
