# URL opening security

Toyoterm exposes OSC 8 hyperlinks through `TerminalSnapshot` and also detects
plain `http://`, `https://`, and `mailto:` URLs in visible terminal text.
Explicit OSC 8 targets take precedence over detected text.

Normally a link is opened only after a user gesture: Control+click on Linux and
Windows, or Command+click on macOS. The explicitly enabled OSC 1337 `OpenURL=:`
path may open a link without a gesture; it is disabled by default and rate
limited as described in the [usage guide](usage.md#osc-url-opening-security).
Before invoking the platform URL handler, both paths reject control characters,
targets longer than 2,048 bytes, missing schemes, and every scheme outside this
allowlist:

- `https`
- `http`
- `mailto`

Terminal output is untrusted, including OSC 8 and OSC 1337 emitted by remote
programs. For that reason `file`, custom application schemes, and executable
schemes are not passed to the operating system. If configurable schemes are added later, they
must use an explicit confirmation dialog showing the complete target and offer
one-time approval separately from a persisted per-scheme permission. The
default allowlist must remain available as a reset option.
