# 0003: Use wgpu for GPU rendering

- Status: Accepted
- Date: 2026-09-02

## Context

The renderer needs predictable terminal-cell composition, text shaping,
selection and cursor overlays, UI chrome, transparency, and high-DPI support on
Linux, macOS, and Windows. Maintaining independent rendering implementations
for Vulkan, Metal, and Direct3D would multiply both code and validation cost.

## Decision

Use wgpu as the cross-platform GPU abstraction and winit for native windows and
event delivery. Keep terminal state independent of GPU resources, and convert
immutable terminal snapshots into render plans and GPU buffers in the render
subsystem.

## Alternatives considered

- Per-platform graphics APIs would allow targeted tuning, but require several
  renderers and substantially more platform-specific lifecycle code.
- A CPU-only renderer would simplify adapter handling, but is a poor fit for
  frequent full-window composition and high-refresh displays.
- A widget toolkit could supply a renderer, but would reduce control over the
  terminal grid and add toolkit-specific layout and input behavior.

## Consequences

- The GPU backend is shared across supported platforms while wgpu selects the
  platform graphics API.
- Adapter selection, surface reconfiguration, device loss, transparency, and
  driver behavior remain explicit failure modes that need tests and graceful
  recovery.
- Terminal parsing and snapshots can be tested without a GPU; GPU lifecycle and
  final pixels require a separate validation layer.

