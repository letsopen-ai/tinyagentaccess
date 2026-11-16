# Desktop permissions

This local server has no authentication. Start it only in the graphical session you intend to control; see SECURITY.md.

## macOS

In System Settings > Privacy & Security > Screen Recording (or Screen & System Audio Recording), enable the terminal/app running tinyagentaccess. In Privacy & Security > Accessibility, enable the same app for input. Quit and restart that app and the server. For a terminal-launched server, permissions generally belong to Terminal/iTerm/the hosting app, not a Rust source file. A headless SSH process may not have access to the logged-in user's desktop. The server checks CGPreflightScreenCaptureAccess and AXIsProcessTrusted and returns the required settings path; it does not open permission prompts itself. Capture also needs an unlocked interactive desktop. CoreGraphics capture is deprecated; test on the intended OS release.

## Wayland

Install `xdg-desktop-portal` and your compositor's backend (`xdg-desktop-portal-gnome` or `xdg-desktop-portal-kde`), with the session D-Bus environment available. The first screenshot, and pointer operations that refresh capture geometry, may request Screenshot permission; first input also requests RemoteDesktop and ScreenCast permission. Approve keyboard and pointer and choose the only attached monitor. The portal cannot force a primary-monitor selection. This version requires a single-monitor session because Screenshot can return the whole desktop without geometry metadata. Denial/missing portal produces a SERVER_ERROR naming the portal and the remedy.

A persistent restore token is saved with mode 0600 at `$XDG_STATE_HOME/tinyagentaccess/portal-restore-token`, falling back to `~/.local/state/tinyagentaccess/portal-restore-token`. Restarting reuses it; compositor policy can still ask again. To reset consent, stop the server, delete that file, revoke sharing in your desktop's permission UI if desired, and restart. Closing sharing ends the input session; next input creates a new one. Screenshot is independently authorized and may prompt again. Accepted owned regular temporary PNGs are removed after reading; unsafe paths are refused and left untouched. Consent operations time out after 60 seconds and return SERVER_ERROR; session info remains available during pending consent. The application directory is 0700; missing/relative state roots disable persistence, and save failures warn without failing input authorization. Screenshot cannot reliably honor include_cursor=false or guarantee include_cursor=true.

Auto mode prefers Wayland even when XWayland sets DISPLAY. Use `platform.linux_backend="x11"` only when intentionally controlling an actual X11 desktop; XTest in XWayland does not control native Wayland applications.

## X11

No additional permission dialog. The server needs DISPLAY and the session's Xauthority cookie. Run as the logged-in desktop user; do not use `xhost +`. The X server must provide XTest; RandR and XFixes improve monitor/cursor selection. Fallback capture is whole root if a primary RandR monitor is unavailable. Existing X11 access already permits broad capture and input.

## Windows

Use an unlocked, logged-in interactive desktop. GDI capture and SendInput do not bypass UAC, the lock screen, secure desktops or input integrity restrictions (UIPI). Run at the same integrity level as the target; SendInput failure returns guidance. Black/protected capture content may be imposed by Windows or individual apps. Process DPI awareness is set once before capture/input; every operation verifies the effective thread context is PER_MONITOR_AWARE_V2 and fails clearly if a conflicting host override prevents this. Primary monitor only; monitors at negative virtual-desktop origins are accounted for in pointer normalization.
