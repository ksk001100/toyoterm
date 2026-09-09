# OSC support

This page tracks Operating System Command (OSC) support in toyoterm. The audit
covers the sequences recognized by the current `alacritty_terminal` 0.26 / VTE
0.15 backend plus common shell and terminal extensions. It does not promise
support for every vendor-private OSC number.

Both BEL and ST termination are accepted unless a row says otherwise.

## Supported

| OSC | Support |
| --- | --- |
| 0, 2 | Set or reset the pane/window title. |
| 4, 104 | Set, query, and reset indexed palette colors. Queries reflect the configured ANSI palette, the xterm 256-color palette, and OSC overrides. |
| 6 | Set the active pane's tab background from iTerm2 red/green/blue brightness components and reset it (together with the title) using `1;bg;*;default`. The override is rendered after all three components have arrived. |
| 7 | Report the working directory with a `file://` URI. |
| 8 | Attach hyperlinks to terminal cells. |
| 9, 777 | Show bounded, rate-limited desktop notifications when `behavior.allow_osc_notifications` is explicitly enabled. OSC `9;4` progress states are independent of that permission and annotate the reporting pane's tab title with normal, error, indeterminate, or warning progress; values outside 0–100 are ignored. |
| 99 | Show OSC 99 title/body notifications, including ID-based chunks and padded or unpadded base64 payloads, and answer capability queries. `always`, `unfocused`, and `invisible` occasions, urgency levels 0–2, and `system`/`silent` sound selection are honored. Positive auto-expiry deadlines are enforced and advertised on Linux/macOS; Windows forwards them as a best effort without advertising `w=1`. Linux also advertises and maps the standard `error`, `warn`/`warning`, `info`, and `question` sounds and maps safe `n=` icon names to the desktop icon theme. Reusing an ID requests replacement from notification backends that support stable IDs. Explicit close is supported and advertised on Linux/macOS; the current Windows backend leaves it unadvertised and treats it as a no-op. Uses the same opt-in, bounds, and rate limit as OSC 9/777. |
| 10, 11, 12 | Set, query, and reset foreground, background, and cursor colors (resets use OSC 110, 111, and 112). Queries reflect the active toyoterm theme and OSC overrides. |
| 21, 30001, 30101 | Set, query, and reset the 256-color palette plus foreground, background, and cursor colors with Kitty's unified color control. Hex, `rgb:`, `rgbi:`, alpha-suffixed values (alpha is currently ignored), and common basic color names are accepted. Push/pop preserves these supported fields on a bounded 10-entry stack; selection and transparent-background color fields are reported as unsupported. |
| 22 | Set, reset, push, pop, and query the mouse-pointer shape using the 30 specified CSS names. Each screen has a bounded 32-entry stack, reset with terminal state; the active shape applies while the pointer is over that pane. |
| 50 | Set the text cursor shape through the `CursorShape=` extension. |
| 52 | Copy UTF-8 text to the system clipboard when `behavior.allow_osc52_copy` is explicitly enabled. Reads stay disabled and decoded payloads are limited to 64 KiB. |
| 133 | Prompt start (`A`), command-line start (`B`), command start (`C`), and command finish (`D[;status]`) markers. Up to 4,096 bounded markers follow scrollback movement; completed command-output zones receive a margin marker colored by exit status, `previous_prompt` / `next_prompt` navigate prompts, and command-output actions select the latest block or cycle through all retained complete `C`–`D` blocks. Markers reset when a resize can reflow lines. |
| 1337 | Inline PNG/JPEG images in the subset documented in [Terminal images](image-protocols.md), `CurrentDir=` working-directory reports, `RemoteHost=user@host` pane metadata, bounded base64 `SetUserVar=name=value` metadata, `CursorShape=0/1/2`, and `SetColors=tab=` using untagged or sRGB three-/six-digit colors plus `default`. |

## Backlog

Priority reflects user impact, interoperability, security constraints, and
whether toyoterm has a UI/state model that can expose the result.

| Priority | OSC | Missing behavior and next step |
| --- | --- | --- |
| P2 | 99 advanced | Alive/close/activation reports, guaranteed replacement on backends that ignore stable IDs, Windows explicit close and guaranteed expiry, transmitted icon data, buttons, and nonstandard platform-specific sounds are not supported or advertised. Named icons are Linux-only. Add the remaining features only with bounded lifecycle state and platform capability mapping. |
| P2 | 1337 privileged operations | Multipart image transfer, downloads (`inline=0`), non-PNG/JPEG formats, `OpenURL`, profile/key-label changes, attention requests, background images, and clipboard-capture variants are not supported. File, focus, URL, or clipboard effects require explicit policy and bounded state. |
| P2 | 5113 | Kitty file transfer is unsupported. It requires an explicit consent UI, destination policy, streaming bounds, cancellation, and safe filename handling before any receive path is enabled. |
| P2 | 66 | Kitty text sizing is ignored. Supporting it requires cell-spanning layout, scaled glyph rendering, cursor movement, selection, and reflow to agree on the occupied cell rectangle. |
| P3 | 1 | Icon-title changes are ignored because toyoterm exposes no separate icon-title surface. |
| P3 | 5, 105, 106 | Xterm special-color tables and enable flags are ignored because toyoterm has no corresponding special-color model. |
| P3 | 13–19, 113–119 | Pointer, Tektronix, and highlight/selection dynamic-color controls are ignored. Pointer/Tektronix colors have no native surface; selection colors currently belong to trusted theme configuration. |
| P3 | 3, 46 | X11 window-property and log-file controls are ignored; they are legacy and OSC 46 would allow untrusted output to request filesystem writes. |
| P3 | 50 (other forms) | Font-changing forms are ignored. Runtime font changes should go through trusted configuration unless a safe use case requires otherwise. |
| P3 | Other iTerm2 1337 metadata | Marks, annotations, badges, shell-integration version, report-variable/cell-size, and captured-output controls have no consumer yet. Add individual bounded subsets only when a native or Ruby-facing use is defined. |

OSC 4/10/11/12 color queries were the first P0 gap found by this audit and are
now implemented. Unsupported OSC input is consumed without being rendered as
text.

The inventory is cross-checked against the current
[xterm control sequences](https://www.invisible-island.net/xterm/ctlseqs/ctlseqs.html),
[Kitty protocol extensions](https://sw.kovidgoyal.net/kitty/protocol-extensions/),
[iTerm2 proprietary sequences](https://iterm2.com/documentation-escape-codes.html),
and [WezTerm escape-sequence reference](https://wezterm.org/escape-sequences.html).
It intentionally distinguishes ignored legacy/private controls from missing
features that have a safe, concrete toyoterm consumer.
