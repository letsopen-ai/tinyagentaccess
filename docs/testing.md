# Desktop backend testing

Ordinary `cargo test --workspace` needs no display. Tests use synthetic frames and recording platforms; they cover key aliases/errors/literal plus, scroll rounding, coordinate viewport/inverse/bounds and native mode, 2×/1.25×/1.5× logical conversion, letterbox pixels, cursor clipping, PNG/JPEG decoding and ordered release on errors. Mock tool integration checks typed arguments, mapping and image bytes. Transport and SDK tests remain from M1.

## Linux X11 automated E2E

Install Xvfb (Ubuntu: `sudo apt-get install xvfb`). Run:

```sh
cargo test -p taa-tools --test x11_e2e -- --ignored --nocapture
```

The test finds a free display in :120…:219, starts a 1920×1080 Xvfb with TCP disabled, creates a colored input-recording x11rb window, and runs real screenshot/click/text/combo/scroll/drag tools. It decodes the 1280×720 PNG and inspects a window pixel, checks mapped root coordinates, observes ASCII and a temporarily mapped CJK keysym, Control state, wheel button counts, drag motion and release. It kills and waits for Xvfb on scope exit. No global DISPLAY mutation is needed. CI installs Xvfb and runs this ignored test explicitly on Ubuntu.

Manual HTTP smoke: start `Xvfb :98 -screen 0 1920x1080x24 -nolisten tcp`, then `DISPLAY=:98 XDG_SESSION_TYPE=x11 cargo run -p tinyagentaccess -- serve`. Use the README initialization/curl flow and call `agentaccess___screenshot` and `agentaccess___left_click` with x=640,y=360. Decode the base64 image: PNG header dimensions must be 1280×720. `DISPLAY=:98 xdotool getmouselocation` should report x=960,y=540. Stop both processes afterwards. Run `/tmp/mcpvenv/bin/python scripts/interop_python_sdk.py` against that server (or a venv with mcp==2.1.1).

## GNOME and KDE Wayland manual acceptance

Run separately on current GNOME and KDE, each with a single monitor and the correct portal backend installed. No compositor/portal exists in the implementation container, so these are not claimed tested.

1. Start inside the desktop with `XDG_SESSION_TYPE=wayland` and both WAYLAND_DISPLAY and DISPLAY set. Default-resolution session info must select `wayland-portal`, not x11; it must return without consent or hanging. In native mode before capture it must return guidance to take a screenshot, rather than inventing dimensions. Repeat with explicit config override and invalid backend config.
2. Call screenshot, deny once, and check actionable SERVER_ERROR. Approve the next call, decode PNG and JPEG, inspect aspect-preserving black bars. Test native size after capture. Check portal temporary PNG removal. Record whether the compositor embeds the cursor despite no exclusion option.
3. Focus a text editor. First input must show sharing consent. Select the primary monitor, allow keyboard/pointer. Verify click/move/drag, double/triple click and right/middle, held down then separate motion/up, each scroll direction at 1/120/180/240 ticks, negative and zero ticks, and ctrl/shift modifiers.
4. Type `ASCII é 中 😀 é\n\t` (send real newline/tab) and inspect the exact text. Check Ctrl+a, Alt+Tab, Super+r, arrows, Return, keypad and a one-second hold. Verify releases after an unavailable key. Some portals cannot synthesize absent Unicode keysyms; record compositor/version and error or loss rather than claiming support.
5. Check restore-token mode is 0600, restart and observe whether consent is reused. Stop sharing in the desktop UI, then call input: a new session must be requested. Delete the token as described in permissions.md and confirm consent is reset.
6. Try two monitors to characterize Screenshot's full-desktop behavior, but do not use this implementation for multi-monitor Wayland input. Compare screenshot physical size vs chosen stream logical size at 100%, 150%, 200% scaling. Record the desktop's portal version and behavior.

## macOS real GUI

Grant the permissions in permissions.md. `cargo test -p taa-platform -- --ignored macos::tests::gui` runs a capture smoke; regular macOS tests check keycodes and permission strings. In an expendable editor, run the same Unicode, mouse, scroll, combo/hold and release checks above. Test Retina and a non-Retina external primary display, native and letterboxed output, cursor true/false, missing permissions, and permission changes after restart. Check actual screenshot orientation/colors and triple-click selection. Test macOS releases that still expose CGDisplayCreateImage. Test linking/running on the self-hosted Mac CI; Linux cross-check only type-checks framework declarations.

## Windows real GUI

`cargo test -p taa-platform -- --ignored windows::tests::gui` needs an unlocked interactive desktop. Ordinary Windows tests check VK tables and portable helpers. In an expendable editor, exercise all tools, Unicode surrogate pairs/CJK/combining text, keypad and modifier holds. Test 100%,125%,150%,200% display scaling, a primary display with secondary monitors at negative origins, PNG/JPEG, cursor inclusion and native dimensions. Check a normal and elevated target, UAC and locked desktop failures. GNU cross-check needs no C compiler; it does not run native APIs.

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
