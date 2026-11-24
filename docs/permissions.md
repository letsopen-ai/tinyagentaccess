# Desktop permissions

This local server has no authentication. Start it only in the graphical session you intend to control; see SECURITY.md.

## macOS

In System Settings > Privacy & Security > Screen Recording (or Screen & System Audio Recording), enable the terminal/app running tinyagentaccess. In Privacy & Security > Accessibility, enable the same app for input. Quit and restart that app and the server. For a terminal-launched server, permissions generally belong to Terminal/iTerm/the hosting app, not a Rust source file. A headless SSH process may not have access to the logged-in user's desktop. The server checks CGPreflightScreenCaptureAccess and AXIsProcessTrusted and returns the required settings path; tinyagentaccess never requests a permission prompt; macOS 15+ may show its own periodic Screen Recording confirmation. Capture also needs an unlocked interactive desktop. Capture uses ScreenCaptureKit on macOS 14+ with the same Screen Recording permission. The binary requires macOS 12.3+; if the screenshot class is absent, it dynamically resolves the legacy CoreGraphics capture API. ScreenCaptureKit failures do not trigger a legacy retry or a permission prompt. Run at most one capturing tinyagentaccess server per desktop session. In a 2026-11-24 isolation run, once a second process attempted a capture after another had already captured in the same session, that attempt timed out after 10 s and both processes kept timing out until restarted (see the README known issues). Test on the intended OS release.

### macOS first run and `doctor`

- Grants belong to the host app that launches the server (Terminal, iTerm, an IDE, a launcher), not to a Rust source file. A new binary path or a new binary may need a new grant.
- The build is not notarized, so macOS may show Gatekeeper / unidentified-developer warnings on first launch.
- Run `tinyagentaccess doctor` (or `tinyagentaccess doctor --json`) from the same host app that will start `tinyagentaccess serve`. It prints the version, absolute binary path and SHA-256, the parent process chain with a likely host app, SSH / non-GUI hints, a remedy naming the host app when known, and the Screen Recording and Accessibility preflight results plus ScreenCaptureKit availability.
- `doctor` only uses the no-prompt preflight APIs (`CGPreflightScreenCaptureAccess`, `AXIsProcessTrusted`). It never prompts, never opens System Settings, never reads `TCC.db`, never changes grants, and does not capture a frame, post input, or bind a port.
- Exit codes: 0 checks passed or not applicable on this OS (Linux and Windows print the common report — version, path, sha256, process chain, hints, remedy — plus a backend summary, with no macOS preflight section); 1 a macOS permission preflight failed; 2 usage or doctor error. A missing ScreenCaptureKit class on an older macOS is informational, not exit 1.
- After granting access, quit and restart the host app and the server. Capture needs an unlocked interactive desktop; SSH or other non-GUI sessions may not see the logged-in desktop.

## Wayland

Install `xdg-desktop-portal` and your compositor's backend (`xdg-desktop-portal-gnome` or `xdg-desktop-portal-kde`), with the session D-Bus environment available. The first screenshot, and pointer operations that refresh capture geometry, may request Screenshot permission; first input also requests RemoteDesktop and ScreenCast permission. Approve keyboard and pointer and choose the only attached monitor. The portal cannot force a primary-monitor selection. This version requires a single-monitor session because Screenshot can return the whole desktop without geometry metadata. Denial/missing portal produces a SERVER_ERROR naming the portal and the remedy.

A persistent restore token is saved with mode 0600 at `$XDG_STATE_HOME/tinyagentaccess/portal-restore-token`, falling back to `~/.local/state/tinyagentaccess/portal-restore-token`. Restarting reuses it; compositor policy can still ask again. To reset consent, stop the server, delete that file, revoke sharing in your desktop's permission UI if desired, and restart. Closing sharing ends the input session; next input creates a new one. Screenshot is independently authorized and may prompt again. Accepted owned regular temporary PNGs are removed after reading; unsafe paths are refused and left untouched. Consent operations time out after 60 seconds and return SERVER_ERROR; session info remains available during pending consent. The application directory is 0700; missing/relative state roots disable persistence, and save failures warn without failing input authorization. Screenshot cannot reliably honor include_cursor=false or guarantee include_cursor=true.

Auto mode prefers Wayland even when XWayland sets DISPLAY. Use `platform.linux_backend="x11"` only when intentionally controlling an actual X11 desktop; XTest in XWayland does not control native Wayland applications.

## X11

No additional permission dialog. The server needs DISPLAY and the session's Xauthority cookie. Run as the logged-in desktop user; do not use `xhost +`. The X server must provide XTest; RandR and XFixes improve monitor/cursor selection. Fallback capture is whole root if a primary RandR monitor is unavailable. Existing X11 access already permits broad capture and input.

## Windows

Use an unlocked, logged-in interactive desktop. GDI capture and SendInput do not bypass UAC, the lock screen, secure desktops or input integrity restrictions (UIPI). Run at the same integrity level as the target; SendInput failure returns guidance. Black/protected capture content may be imposed by Windows or individual apps. Process DPI awareness is set once before capture/input; every operation verifies the effective thread context is PER_MONITOR_AWARE_V2 and fails clearly if a conflicting host override prevents this. Primary monitor only; monitors at negative virtual-desktop origins are accounted for in pointer normalization.
