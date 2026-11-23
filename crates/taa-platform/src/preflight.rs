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

    /// Normalizes CRLF (Windows checkouts) to LF so marker searches are
    /// line-ending independent.
    fn lf(src: &str) -> String {
        src.replace("\r\n", "\n")
    }

    /// Byte offset of `needle` in `hay`, with a descriptive failure.
    fn find_marker(hay: &str, needle: &str, what: &str) -> usize {
        hay.find(needle)
            .unwrap_or_else(|| panic!("{what}: marker `{needle}` not found in scanned source"))
    }

    /// Returns the text from `needle` up to (not including) the next `end`.
    fn block_from<'a>(src: &'a str, needle: &str, end: &str, what: &str) -> &'a str {
        let start = find_marker(src, needle, what);
        let rest = &src[start..];
        let len = rest
            .find(end)
            .unwrap_or_else(|| panic!("{what}: terminator `{end:?}` not found after `{needle}`"));
        &rest[..len]
    }

    /// Returns the exact `sig` text if present (FFI declaration check).
    fn signature<'a>(src: &'a str, sig: &str, what: &str) -> &'a str {
        let at = find_marker(src, sig, what);
        &src[at..at + sig.len()]
    }

    /// The doctor preflight path (this file plus the macOS wrappers it calls)
    /// must never reach request/prompt APIs or the TCC database.
    #[test]
    fn preflight_path_has_no_prompt_apis() {
        let own = lf(include_str!("preflight.rs"));
        let own = own.split("#[cfg(test)]").next().unwrap();
        let mac = lf(include_str!("macos.rs"));
        let cap = lf(include_str!("macos_capture.rs"));
        let begin = find_marker(&mac, "// doctor-preflight-begin", "macos.rs");
        let end = find_marker(&mac, "// doctor-preflight-end", "macos.rs");
        assert!(begin < end, "doctor-preflight markers are out of order");
        let wrappers = &mac[begin..end];
        assert!(wrappers.contains("CGPreflightScreenCaptureAccess"));
        assert!(wrappers.contains("AXIsProcessTrusted()"));
        // FFI bindings the wrappers call, and the class probe (not `capture`).
        let ffi_mac = &mac[..begin];
        let ffi_a = signature(
            ffi_mac,
            "fn CGPreflightScreenCaptureAccess() -> bool;",
            "macos.rs",
        );
        let ffi_b = signature(ffi_mac, "fn AXIsProcessTrusted() -> bool;", "macos.rs");
        let probe = block_from(
            &cap,
            "pub(crate) fn available() -> bool {",
            "\n    }\n",
            "macos_capture.rs",
        );
        assert!(probe.contains("AnyClass::get"));
        for text in [own, wrappers, ffi_a, ffi_b, probe] {
            for banned in FORBIDDEN {
                assert!(!text.contains(banned), "forbidden `{banned}`");
            }
        }
    }

    /// Guards the scan itself: every needle must be found when the included
    /// sources use CRLF, and the probe must stop at `available()`'s end.
    #[test]
    fn scan_needles_survive_crlf_sources() {
        for (name, src) in [
            ("macos.rs", include_str!("macos.rs")),
            ("macos_capture.rs", include_str!("macos_capture.rs")),
        ] {
            let crlf = lf(src).replace('\n', "\r\n");
            let norm = lf(&crlf);
            if name == "macos.rs" {
                find_marker(&norm, "// doctor-preflight-begin", name);
                find_marker(&norm, "// doctor-preflight-end", name);
                signature(&norm, "fn CGPreflightScreenCaptureAccess() -> bool;", name);
                signature(&norm, "fn AXIsProcessTrusted() -> bool;", name);
            } else {
                let probe = block_from(
                    &norm,
                    "pub(crate) fn available() -> bool {",
                    "\n    }\n",
                    name,
                );
                assert!(probe.contains("AnyClass::get"));
                assert!(!probe.contains("fn capture"), "probe leaked into capture()");
            }
            // The raw CRLF text would not match LF-only patterns; normalization is required.
            assert!(!norm.contains('\r'));
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
