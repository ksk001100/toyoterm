#!/bin/sh
set -eu

: "${DISPLAY:?X11 smoke test requires an X11 display}"

unset WAYLAND_DISPLAY
export LIBGL_ALWAYS_SOFTWARE=1

lavapipe_icd=$(find /usr/share/vulkan/icd.d -name 'lvp_icd*.json' -print -quit)
if [ -z "$lavapipe_icd" ]; then
  echo "X11 smoke test: Lavapipe Vulkan driver was not found" >&2
  exit 1
fi
export VK_DRIVER_FILES=$lavapipe_icd
export VK_ICD_FILENAMES=$lavapipe_icd

openbox_log=$(mktemp)
openbox >"$openbox_log" 2>&1 &
openbox_pid=$!
cleanup() {
  kill "$openbox_pid" 2>/dev/null || true
  wait "$openbox_pid" 2>/dev/null || true
  rm -f "$openbox_log"
}
trap cleanup EXIT HUP INT TERM

attempt=0
while ! xprop -root _NET_SUPPORTING_WM_CHECK 2>/dev/null | grep -q 'window id'; do
  attempt=$((attempt + 1))
  if [ "$attempt" -ge 100 ]; then
    cat "$openbox_log" >&2
    exit 1
  fi
  sleep 0.1
done

cargo run --locked -- gui-smoke-test
