# Keyboard vocabulary

Names are case-insensitive (`Return`, `RETURN`, `return` agree); single printable characters preserve case (`A` includes Shift where needed). Combos use `+`, press left to right and release right to left. `key` releases immediately; `hold_key` keeps the combo down for 1–30 seconds. Releases are attempted after returned input errors. Examples: `ctrl+c`, `super+r`, `alt+F4`, `alt+Tab`, `shift+Prior`.

Write a literal plus as `plus`, `ctrl+plus`, or `ctrl++`. A trailing separator also denotes plus (`ctrl+`); use the unambiguous `plus` spelling in generated commands. Empty combo/components are errors. Repeated keys are deduplicated, preserving first occurrence. `modifiers` uses the same parser but accepts only modifier names; `ctrl+a` is not a valid modifiers field. Unknown names produce `CLIENT_ERROR: unknown key <name>` before dispatch, including inside execute_sequence preflight. A recognized key missing on the actual OS/keymap returns SERVER_ERROR rather than substituting another character.

Modifier aliases:

| Canonical | Aliases | macOS | X11 / portal keysym | Windows VK |
|---|---|---:|---|---|
| ctrl | control | 59 | Control_L (0xffe3) | CONTROL (0x11) |
| alt | option | 58 | Alt_L (0xffe9) | MENU (0x12) |
| shift | — | 56 | Shift_L (0xffe1) | SHIFT (0x10) |
| super | cmd, command, meta, win, windows | 55 (⌘) | Super_L (0xffeb) | LWIN (0x5b) |
| fn | — | 63 | 0x1008ff2b, if mapped | unavailable |

Special keys (numbers in the macOS column are CG hardware keycodes). X11/portal symbols retain standard X keysym values; aliases are resolved before the backend.

| Name | Aliases | macOS code | X11 / portal | Windows VK |
|---|---|---:|---|---|
| enter | return | 36 | Return 0xff0d | RETURN 0x0d |
| tab | — | 48 | Tab 0xff09 | TAB 0x09 |
| esc | escape | 53 | Escape 0xff1b | ESCAPE 0x1b |
| space | literal space | 49 | space 0x20 | SPACE 0x20 |
| backspace | BackSpace | 51 | BackSpace 0xff08 | BACK 0x08 |
| delete | del | 117 | Delete 0xffff | DELETE 0x2e |
| insert | — | 114 (Help) | Insert 0xff63 | INSERT 0x2d |
| home | — | 115 | Home 0xff50 | HOME 0x24 |
| end | — | 119 | End 0xff57 | END 0x23 |
| pageup | page_up, prior | 116 | Prior 0xff55 | PRIOR 0x21 |
| pagedown | page_down, next | 121 | Next 0xff56 | NEXT 0x22 |
| left | arrowleft | 123 | Left 0xff51 | LEFT 0x25 |
| up | arrowup | 126 | Up 0xff52 | UP 0x26 |
| right | arrowright | 124 | Right 0xff53 | RIGHT 0x27 |
| down | arrowdown | 125 | Down 0xff54 | DOWN 0x28 |
| capslock | caps_lock | 57 | Caps_Lock 0xffe5 | CAPITAL 0x14 |
| numlock | num_lock | 71 (Clear) | Num_Lock 0xff7f | NUMLOCK 0x90 |
| scrolllock | scroll_lock | 107 (F14) | Scroll_Lock 0xff14 | SCROLL 0x91 |
| printscreen | print | 105 (F13) | Print 0xff61 | SNAPSHOT 0x2c |
| pause | — | 113 (F15) | Pause 0xff13 | PAUSE 0x13 |
| menu | — | 110 | Menu 0xff67 | APPS 0x5d |
| F1…F12 | case-insensitive | 122,120,99,118,96,97,98,100,101,109,103,111 | 0xffbe…0xffc9 | F1…F12 (0x70…0x7b) |
| F13…F20 | — | 105,107,113,106,64,79,80,90 | 0xffca…0xffd1 | F13…F20 (0x7c…0x83) |
| F21…F24 | — | unavailable | 0xffd2…0xffd5 | F21…F24 (0x84…0x87) |

Keypad vocabulary:

| Name | macOS | X11 / portal keysym | Windows VK |
|---|---|---|---|
| KP_0…KP_9 | 82,83,84,85,86,87,88,89,91,92 | KP_0…KP_9 0xffb0…0xffb9 | NUMPAD0…9 0x60…0x69 |
| KP_Enter | 76 | KP_Enter 0xff8d | RETURN |
| KP_Add / KP_Subtract | 69 / 78 | 0xffab / 0xffad | ADD / SUBTRACT |
| KP_Multiply / KP_Divide | 67 / 75 | 0xffaa / 0xffaf | MULTIPLY / DIVIDE |
| KP_Decimal / KP_Separator | 65 / 65 | 0xffae / 0xffac | DECIMAL / SEPARATOR |
| KP_Equal | 81 | 0xffbd | OEM_PLUS |
| KP_Space / KP_Tab | 49 / 48 | 0xff80 / 0xff89 | SPACE / TAB |
| KP_F1…KP_F4 | same as F1…F4 | 0xff91…0xff94 | F1…F4 |
| KP_Home, KP_Left, KP_Up, KP_Right, KP_Down | ordinary navigation codes | 0xff95…0xff99 | ordinary navigation VKs |
| KP_Prior / KP_Page_Up | 116 | 0xff9a | PRIOR |
| KP_Next / KP_Page_Down | 121 | 0xff9b | NEXT |
| KP_End / KP_Begin | 119 / 71 | 0xff9c / 0xff9d | END / CLEAR |
| KP_Insert / KP_Delete | 114 / 117 | 0xff9e / 0xff9f | INSERT / DELETE |

Cheap media support:

| Name | Alias | macOS | X11 / portal keysym | Windows VK |
|---|---|---|---|---|
| volumeup | XF86AudioRaiseVolume | 72 | 0x1008ff13 | VOLUME_UP |
| volumedown | XF86AudioLowerVolume | 73 | 0x1008ff11 | VOLUME_DOWN |
| mute | XF86AudioMute | 74 | 0x1008ff12 | VOLUME_MUTE |
| play | XF86AudioPlay | unavailable | 0x1008ff14 | MEDIA_PLAY_PAUSE |

Single printable characters use their Unicode character keysym on X11/portals (Latin-1 directly; others `0x01000000 | codepoint`). Named punctuation aliases are plus, minus, equal, slash. X11 looks up the current keyboard mapping; Windows uses VkKeyScanW; macOS combos use US hardware positions, including shifted punctuation. Availability and keypad distinctions depend on the keyboard layout and NumLock. There is no arbitrary X keysym database: documented names above are supported, unknown names are rejected. On macOS F13/F14/F15/Clear are the conventional hardware approximations for PC-only keys, not an assertion of identical OS behavior.

For arbitrary Unicode strings use **type_text**, not a character combo. It bypasses layout on macOS (CGEvent Unicode strings) and Windows (UTF-16 KEYEVENTF_UNICODE). X11 temporarily remaps the highest keycode for absent symbols, then restores it. This emits key events, so applications must interpret Unicode keysyms correctly. Wayland uses portal keysym notifications; support for non-keymap characters requires compositor validation. No clipboard data is read or written. Newline and tab generate Return and Tab. Events pause 5 ms between Unicode scalars; macOS chunks are one scalar (one/two UTF-16 units). Emoji, CJK and combining marks are counted as Unicode scalars for the 10,000-character limit, rather than bytes or graphemes.
