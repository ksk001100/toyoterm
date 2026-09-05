# 0007: Use GPUI for the desktop GUI

Status: Accepted. Supersedes [0003](0003-use-wgpu.md).

## Decision

Use the published GPUI 0.2.2 crate, pinned in the workspace, for native window
lifecycle, foreground scheduling, input/IME, text shaping, and GPU presentation.
Remove the application's winit loop and the wgpu/glyphon renderer. Use GPUI's
standard APIs rather than maintaining a fork for unavailable platform controls.

`toyoterm-app` owns a GPUI root entity and all mutable terminal/mux/PTY state on
the main thread. Worker messages use a FIFO async channel; the foreground task
processes bounded batches and requests a coalesced render. The named mruby
thread and serialized request/completion protocol are unchanged.

`toyoterm-render` retains the backend-independent layout and color contracts,
and builds a retained scene painted through GPUI. Terminal cells use explicit
grid positions so fallback glyph advances cannot displace subsequent columns.
GPUI owns device recovery and presentation; the app no longer manages surfaces.

## Compatibility

Physical-position bindings and distinct application-keypad encoding are not
available through this GPUI release. Bindings use logical keys; `physical` and
raw `PHYSICAL:` bindings fail configuration validation with rollback. Setting
`window.always_on_top` to true likewise fails validation. Window decorations,
resizability, dimensions and minimum dimensions are creation options, applied
on the next launch. Font, color, opacity, layout, title and key-binding changes
still apply after successful configuration transactions. Native maximize,
minimize and fullscreen operations use GPUI's platform implementation.

These changes deliberately prefer standard GPUI over a parallel legacy GUI.
Linux requires GPUI's X11/Wayland and font development libraries; macOS and
Windows use GPUI's native backends. Platform-specific IME, transparency and
window-manager behavior require the interactive platform validation checklist.

## Validation

Keep pure layout/color/scene regressions and script rollback tests. Replace
legacy GPU surface tests with GPUI scene coverage and a native smoke command
that paints a frame before quitting. CI builds all three desktop platforms;
Linux runs separate X11 and Wayland smoke jobs.
