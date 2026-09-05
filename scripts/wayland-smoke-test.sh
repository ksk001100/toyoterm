#!/bin/sh
set -eu

: "${DISPLAY:?Wayland smoke test requires an X11 display for nested Weston}"

runtime_directory=$(mktemp -d)
chmod 700 "$runtime_directory"
export XDG_RUNTIME_DIR=$runtime_directory
export WAYLAND_DISPLAY=wayland-toyoterm-ci

# GPUI requires a wl_seat, which Weston's headless backend does not advertise.
# Nest Weston under Xvfb so the Wayland client still has a virtual input seat.
weston --backend=x11-backend.so --socket="$WAYLAND_DISPLAY" --idle-time=0 \
  >"$runtime_directory/weston.log" 2>&1 &
weston_pid=$!
cleanup() {
  kill "$weston_pid" 2>/dev/null || true
  wait "$weston_pid" 2>/dev/null || true
  rm -rf "$runtime_directory"
}
trap cleanup EXIT HUP INT TERM

socket_path="$runtime_directory/$WAYLAND_DISPLAY"
attempt=0
while [ ! -S "$socket_path" ]; do
  attempt=$((attempt + 1))
  if [ "$attempt" -ge 100 ]; then
    cat "$runtime_directory/weston.log" >&2
    exit 1
  fi
  sleep 0.1
done

cargo run --locked -- gui-smoke-test
