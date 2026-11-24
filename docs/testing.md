# Desktop backend testing

Ordinary `cargo test --workspace` needs no display. Tests use synthetic frames and recording platforms; they cover key aliases/errors/literal plus, scroll rounding, coordinate viewport/inverse/bounds and native mode, 2×/1.25×/1.5× logical conversion, letterbox pixels, cursor clipping, PNG/JPEG decoding and ordered release on errors. Mock tool integration checks typed arguments, mapping and image bytes. Transport and SDK tests remain from M1.

## Linux X11 automated E2E

Install Xvfb (Ubuntu: `sudo apt-get install xvfb`). Run:

```sh
cargo test -p taa-tools --test x11_e2e -- --ignored --nocapture
```

The test finds a free display in :120…:219, starts a 1920×1080 Xvfb with TCP disabled, creates a colored input-recording x11rb window, and runs real screenshot/click/text/combo/scroll/drag tools. It decodes the 1280×720 PNG and checks pixels on both sides of each window edge, checks independent expected root coordinates, observes ASCII and a temporarily mapped CJK keysym, Control state, wheel button counts, drag motion and release, complete spare-keycode restoration, cleanup after an unavailable combo, release after a mid-hold keymap change, and reconnect after X server restart. It kills and waits for Xvfb on scope exit. No global DISPLAY mutation is needed. CI installs Xvfb and runs this ignored test explicitly on Ubuntu.

Manual HTTP smoke: start `Xvfb :98 -screen 0 1920x1080x24 -nolisten tcp`, then `DISPLAY=:98 XDG_SESSION_TYPE=x11 cargo run -p tinyagentaccess -- serve`. Use the README initialization/curl flow and call `agentaccess___screenshot` and `agentaccess___left_click` with x=640,y=360. Decode the base64 image: PNG header dimensions must be 1280×720. `DISPLAY=:98 xdotool getmouselocation` should report x=960,y=540. Stop both processes afterwards. Run `/tmp/mcpvenv/bin/python scripts/interop_python_sdk.py` against that server (or a venv with mcp==2.1.1).

## GNOME and KDE Wayland manual acceptance

Run separately on current GNOME and KDE, each with a single monitor and the correct portal backend installed. No compositor/portal exists in the implementation container, so these are not claimed tested.

1. Start inside the desktop with `XDG_SESSION_TYPE=wayland` and both WAYLAND_DISPLAY and DISPLAY set. Default-resolution session info must select `wayland-portal`, not x11; it must return without consent or hanging. In native mode before capture it must return guidance to take a screenshot, rather than inventing dimensions. While a consent dialog is pending, verify session info still returns promptly; leave consent unanswered and check timeout guidance after 60 seconds. Wait intentionally retains desktop ordering. Repeat with explicit config override and invalid backend config.
2. Call screenshot, deny once, and check actionable SERVER_ERROR. Approve the next call, decode PNG and JPEG, inspect aspect-preserving black bars. Test native size after capture. Check portal temporary PNG removal. Record whether the compositor embeds the cursor despite no exclusion option.
3. Focus a text editor. First input must show sharing consent. Select the primary monitor, allow keyboard/pointer. Verify click/move/drag, double/triple click and right/middle, held down then separate motion/up, each scroll direction at 1/120/180/240 ticks, negative and zero ticks, and ctrl/shift modifiers.
4. Type `ASCII é 中 😀 é\n\t` (send real newline/tab) and inspect the exact text. Check Ctrl+a, Alt+Tab, Super+r, arrows, Return, keypad and a one-second hold. Verify releases after an unavailable key. Some portals cannot synthesize absent Unicode keysyms; record compositor/version and error or loss rather than claiming support.
5. Check restore-token mode is 0600, restart and observe whether consent is reused. Stop sharing in the desktop UI, then call input: a new session must be requested. Delete the token as described in permissions.md and confirm consent is reset.
6. Try two monitors to characterize Screenshot's full-desktop behavior, but do not use this implementation for multi-monitor Wayland input. Compare screenshot physical size vs chosen stream logical size at 100%, 150%, 200% scaling. Record the desktop's portal version and behavior.

### Known limitation: headless GNOME Wayland (v0.1.2, Ubuntu 24.04)

On 2026-11-24, tinyAgentAccess v0.1.2 was tested on Ubuntu 24.04 in a headless GNOME Wayland session (`gnome-shell --wayland --headless` with a virtual monitor) already remoted by GNOME Remote Desktop. The seated, interactive GNOME and KDE acceptance checks above remain unvalidated by this test.

- Session info returned `wayland-portal` promptly without a consent prompt. Screenshots succeeded with existing screenshot and screencast grants in the PermissionStore.
- RemoteDesktop input returned `SERVER_ERROR` after the 60-second portal approval timeout. `xdg-desktop-portal-gnome` logged a window association failure with an empty `parent_window` and, on session close, `Failed to stop screen cast session: Session not started`. No restore token was written and the PermissionStore had no remote-desktop entry.
- A standalone client following the [RemoteDesktop portal specification](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html) reproduced it in the same session, with `Request.Response` subscribed before each call, `persist_mode=0`, empty `parent_window`, and a 120-second wait. `CreateSession`, `SelectDevices` and `ScreenCast.SelectSources` each returned Response status 0 immediately. `RemoteDesktop.Start` reached `xdg-desktop-portal-gnome`, but no `Start` `Request.Response` was emitted for the full 120 seconds. `NotifyPointer*` was never attempted.

The standalone reproduction shows that the portal failed to emit the `Start` response within the observation window in this setup. A conflict involving GNOME Remote Desktop and Mutter’s ScreenCast backend is suspected and remains unconfirmed. Disconnecting GNOME Remote Desktop was not tested. No workaround is supported. For automated Linux testing, use X11/Xvfb as described above.

## macOS real GUI

Grant the permissions in permissions.md. First run `tinyagentaccess doctor` (optionally `--json`) from the same host app and confirm both preflight results are true; it is read-only and never prompts, and exits 1 when a preflight fails. Doctor output on a real Mac is manual validation; Linux CI only exercises it with fakes and the Linux backend summary. `cargo test -p taa-platform -- --ignored macos::tests::gui` runs a capture smoke; regular macOS tests check keycodes and permission strings. In an expendable editor, run the same Unicode, mouse, scroll, combo/hold and release checks above. Test Retina and a non-Retina external primary display, native and letterboxed output, cursor true/false, missing permissions, and permission changes after restart. Check actual screenshot orientation/colors and triple-click selection. Change primary display and scaling without restarting; verify native dimensions and click the right/bottom capture edges. Test cmd+j/cmd+k/cmd+l and uppercase N/M. On macOS 14+, confirm the backend name is `macos-screencapturekit`; exercise ScreenCaptureKit at native resolution and 1280×720, cursor on/off, denied Screen Recording permission, and display-scale changes. Re-check the previous disposable Tk-window anomaly: legacy capture once showed the title bar but a blank white client area although text widgets were visible locally. Verify the new full-display filter captures the client contents, desktop, dock and menu bar without Tk-specific workarounds. Test the `macos-coregraphics` fallback only on macOS 12.3–13.x where SCScreenshotManager is absent; there is no force-legacy flag. Verify legacy-unavailable, display-missing and timeout errors where feasible, and ensure ordinary SCK errors never fall back. Standard GitHub-hosted macOS CI links and runs ordinary tests; Linux cross-check only type-checks framework declarations. Real GUI validation remains manual. Do not run two capturing servers in one desktop session during validation; a second capturing process was observed to trigger the 10 s ScreenCaptureKit timeout in both processes until restart.

## Windows real GUI

`cargo test -p taa-platform -- --ignored windows::tests::gui` needs an unlocked interactive desktop. Ordinary Windows tests check VK tables and portable helpers. In an expendable editor, exercise all tools, Unicode surrogate pairs/CJK/combining text, keypad and modifier holds. Test 100%,125%,150%,200% display scaling, a primary display with secondary monitors at negative origins, PNG/JPEG, cursor inclusion and native dimensions. Check a normal and elevated target, UAC and locked desktop failures. Check AltGr-layout characters requiring Ctrl+Alt, shift+A, and cursor positions against GetCursorPos at edges (included in the ignored GUI test). Verify a conflicting thread DPI context produces an error. GNU cross-check needs no C compiler; it does not run native APIs.

## Checks used for M2

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check --workspace --all-targets --target x86_64-pc-windows-gnu
cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings
cargo check --workspace --all-targets --target aarch64-apple-darwin
cargo clippy --workspace --all-targets --target aarch64-apple-darwin -- -D warnings
cargo test -p taa-tools --test x11_e2e -- --ignored --nocapture
```

The Linux cross-target CI job includes macOS ARM check and clippy for fork PRs; it only type-checks without an SDK, never links or runs macOS code. Standard GitHub-hosted macOS CI links and runs ordinary tests. Native macOS/Windows mapping audits compile in cross-checks but execute only on their own OS runners. Mac printable ASCII tests also run on Linux. See the external review disposition for exact local validation results and remaining platform blockers.

## M3 coverage

Sequence tests cover exact rejection text and JSON rows, count/wait/text boundaries (Unicode scalars), prefixed and SAMPLE short names, nesting/unknown/gated actions, shared validators, success timing/image shape, paused-clock wait deadlines, skipped stabilization after wait, first failure and terminal capture failure. Storage tests use temporary directories for PNG/JPEG layout, Unix permissions, disabled saving, invalid paths/IDs, directory symlinks, nonfatal failures and terminal images. HTTP tests check modern ID markers on sequence success/rejection/failure, repeated close, post-close refusal and held-button release. Existing transport tests cover JSON-RPC error.data.mcpSessionId.

The ignored Xvfb test now also runs move_pointer + left_click + wait + key and checks the terminal image and summary. The Python SDK script calls sequence success and rejection in both legacy/stateful and modern/stateless modes; it requires a working desktop backend for the terminal image. Start the isolated Xvfb and HTTP server described above, run `/tmp/mcpvenv/bin/python scripts/interop_python_sdk.py`, then stop both processes. Cross-target check/clippy commands above remain required for M3.

## M4 forwarding

`cargo test --workspace` runs portable Rust stdio fixtures, with no Python dependency.
`taa-forward` covers encoding/shape rejection, ignored keys, config order, names,
manifest passthrough, parallel calls, pagination, empty/failed lists, initialization
and request timeouts, cancellation, downstream errors/crashes and actual child reaping.
`taa-mcp` exercises forwarding through stateful and modern stateless HTTP, including
NEW, schema injection, forwarding-only visibility, disabled forwarding, and closure.
The fixture binaries share one source and are deliberately normal Cargo binaries so
integration tests and SDK verification can launch exactly the same implementation.

For the official Python SDK (`mcp==2.1.1`) run `scripts/verify_m4.sh`; it builds the
server/fixture, creates absolute-path TOML and JSON configs in a temporary directory,
starts an isolated Xvfb display and server, and runs both SDK scripts. It prints PASS
lines for both protocol modes and exits nonzero on failure. Requires Xvfb and a Python
interpreter with mcp installed (override `MCP_PYTHON`, default `/tmp/mcpvenv/bin/python`).
For an already running configured server:

```sh
MCP_PYTHON=/tmp/mcpvenv/bin/python
"$MCP_PYTHON" scripts/interop_forwarding.py --url http://127.0.0.1:8765/mcp
```

The redirection entry must be named `fixture`, command must be the absolute path to
`target/debug/taa-fake-mcp-server`, and args must be `["good"]`. Enable forwarding in
TOML and point forwarding.config_path at that JSON file. Keep vision/input enabled
when running the original `interop_python_sdk.py`, which also tests screenshots.
