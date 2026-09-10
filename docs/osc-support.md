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
| 1 | Store a bounded icon title independently as pane metadata. It is available to trusted Ruby as `Pane#icon_title` and through OSC 1337 `ReportVariable=session.terminalIconName`; toyoterm has no separate visual icon-title surface. |
| 4, 104 | Set, query, and reset indexed palette colors. Queries reflect the configured ANSI palette, the xterm 256-color palette, and OSC overrides. |
| 6 | Set the active pane's tab background from iTerm2 red/green/blue brightness components and reset it (together with the title) using `1;bg;*;default`. The override is rendered after all three components have arrived. |
| 7 | Report the working directory with a `file://` URI. |
| 8 | Attach hyperlinks to terminal cells. |
| 9, 777 | Show bounded, rate-limited desktop notifications when `behavior.allow_osc_notifications` is explicitly enabled. OSC `9;4` progress states are independent of that permission and annotate the reporting pane's tab title with normal, error, indeterminate, or warning progress; values outside 0–100 are ignored. |
| 99 | Show OSC 99 title/body notifications, including ID-based chunks and padded or unpadded base64 payloads, and answer capability queries. `always`, `unfocused`, and `invisible` occasions, urgency levels 0–2, and `system`/`silent` sound selection are honored. Positive auto-expiry deadlines are enforced and advertised on Linux/macOS; Windows forwards them as a best effort without advertising `w=1`. Linux also advertises and maps the standard `error`, `warn`/`warning`, `info`, and `question` sounds and maps safe `n=` icon names to the desktop icon theme. On Linux/macOS, reusing an ID explicitly closes the tracked notification before showing its replacement, including on services that ignore replacement IDs. Explicit close is supported and advertised on Linux/macOS; the current Windows backend leaves it unadvertised and treats it as a no-op. Uses the same opt-in, bounds, and rate limit as OSC 9/777. |
| 10, 11, 12 | Set, query, and reset foreground, background, and cursor colors (resets use OSC 110, 111, and 112). Queries reflect the active toyoterm theme and OSC overrides. |
| 17, 19, 117, 119 | Set, query, and reset xterm selection background and foreground colors. Explicit colors override the configured selection rendering; resets restore it. Queries use the configured selection background or foreground as the legacy protocol fallback when the corresponding color is dynamic. |
| 21, 30001, 30101 | Set, query, and reset the 256-color palette plus foreground, background, cursor, cursor-text, and selection colors with Kitty's unified color control. Hex, `rgb:`, `rgbi:`, alpha-suffixed values (alpha is currently ignored), and common basic color names are accepted. Explicit cursor-text and selection colors and bare-key resets are supported; queries return an empty value for dynamic cursor-text or selection-foreground state. Empty-value dynamic assignments and transparent-background colors remain unsupported. Push/pop preserves all supported fields on a bounded 10-entry stack. |
| 22 | Set, reset, push, pop, and query the mouse-pointer shape using the 30 specified CSS names. Each screen has a bounded 32-entry stack, reset with terminal state; the active shape applies while the pointer is over that pane. |
| 50 | Set the text cursor shape through the `CursorShape=` extension. |
| 52 | Copy UTF-8 text to the system clipboard when `behavior.allow_osc52_copy` is explicitly enabled. Reads stay disabled and decoded payloads are limited to 64 KiB. |
| 133 | Prompt start (`A`), command-line start (`B`), command start (`C`), and command finish (`D[;status]`) markers. Up to 4,096 bounded markers follow scrollback movement; completed command-output zones receive a margin marker colored by exit status, `previous_prompt` / `next_prompt` navigate prompts, and command-output actions select the latest block or cycle through all retained complete `C`–`D` blocks. Markers reset when a resize can reflow lines. |
| 1337 | Inline PNG/JPEG/GIF/BMP/WebP images (the first animated frame is displayed), including bounded `MultipartFile`/`FilePart`/`FileEnd` transfers, in the subset documented in [Terminal images](image-protocols.md); `SetMark` locations retained with scrollback and navigated by `previous_mark` / `next_mark`; `ClearScrollback` history clearing; `HighlightCursorLine=yes/no` cursor guides; opt-in `RequestAttention=yes/once/no` platform attention hints; opt-in, bounded and rate-limited `OpenURL=:` requests restricted to `https`, `http`, and `mailto`; opt-in one-shot `Copy=:` and unnamed `CopyToClipboard=` / `EndCopy` clipboard writes with the OSC 52 64 KiB bound; bounded base64 `SetBadgeFormat=` pane badges with safe `session.*` / `user.*` variable interpolation; `CurrentDir=` working-directory reports; `RemoteHost=user@host` and bounded `ShellIntegrationVersion=version;shell` pane metadata; bounded base64 `SetUserVar=name=value` metadata; `CursorShape=0/1/2`; `ReportCellSize` replies with logical height, width, and display scale; bounded `ReportVariable` queries for `session.name`, `session.terminalIconName`, `session.columns`, `session.rows`, `session.path`, `session.shell`, `session.hostname`, `session.username`, and `session.user.*` (unknown or unset variables return an empty value); and `SetColors=` for normal/bold foreground, background, cursor background/foreground, link, underline, selection foreground/background, ANSI 16-color palette, and tab using untagged, `rgb:`, `srgb:`, or `p3:` three-/six-digit colors (`tab=default` resets the tab color). Display P3 input is converted to sRGB for rendering. Dynamic defaults, palette entries, cursor text, hyperlinks, underlines, and selections are reflected in rendering. The bounded color stack preserves all supported session colors. Ruby-set pane badges take display precedence over OSC badges. |
| 21337 | Set or independently clear the active pane's bounded iTerm2 session-status text, `#rrggbb` status color, and `#rrggbb` tab indicator. The active pane supplies the tab presentation; status text is rendered after the tab title and progress annotation. Unknown fields are ignored, while malformed recognized fields reject the update atomically. |

## Backlog

Priority reflects user impact, interoperability, security constraints, and
whether toyoterm has a UI/state model that can expose the result.

| Priority | OSC | Missing behavior and next step |
| --- | --- | --- |
| P2 | 99 advanced | Alive/close/activation reports, Windows explicit close and guaranteed expiry, transmitted icon data, buttons, and nonstandard platform-specific sounds are not supported or advertised. Named icons are Linux-only. Add the remaining features only with bounded lifecycle state and platform capability mapping. |
| P2 | 1337 privileged operations | Downloads (`inline=0`), uploads (`RequestUpload`), image animation and remaining image formats, profile/key-label changes, the cursor-local `RequestAttention=fireworks` effect, background images, and named `rule`/`find`/`font` clipboard destinations are not supported. File, focus, URL, or clipboard effects require explicit policy, user consent where applicable, and bounded state. |
| P2 | 1337 named color presets | `SetColors=preset=name` is not supported because iTerm2 preset names are installation-specific and toyoterm has no matching per-session preset registry. |
| P2 | 5113 | Kitty file transfer is unsupported. It requires an explicit consent UI, destination policy, streaming bounds, cancellation, and safe filename handling before any receive path is enabled. |
| P2 | 66 | Kitty text sizing is ignored. Supporting it requires cell-spanning layout, scaled glyph rendering, cursor movement, selection, and reflow to agree on the occupied cell rectangle. |
| P3 | 5, 105, 106 | Xterm special-color tables and enable flags are ignored because toyoterm has no corresponding special-color model. |
| P3 | 13–16, 18, 113–116, 118 | Pointer and Tektronix dynamic-color controls are ignored because toyoterm has no corresponding native surface. |
| P3 | 21 dynamic/extra colors | Empty-value dynamic assignments, visual-bell color, and transparent-background color slots are unsupported. Dynamic selection background requires reverse-video selection rendering, while the other fields need corresponding native effects. |
| P3 | 3, 46 | X11 window-property and log-file controls are ignored; they are legacy and OSC 46 would allow untrusted output to request filesystem writes. |
| P3 | 50 (other forms) | Font-changing forms are ignored. Runtime font changes should go through trusted configuration unless a safe use case requires otherwise. |
| P3 | Other iTerm2 1337 metadata | Annotations, Unicode-width version stacks, captured-output controls, and custom control sequences have no compatible consumer yet. Add individual bounded subsets only when a native or Ruby-facing use is defined. |

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
