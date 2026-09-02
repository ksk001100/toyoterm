#!/bin/sh
set -eu

prefix=${TOYOTERM_PREFIX:-"${HOME:?HOME is not set}/.local"}
if [ "${1:-}" = "--prefix" ]; then
  if [ "$#" -ne 2 ] || [ -z "$2" ]; then
    echo "usage: uninstall.sh [--prefix PREFIX]" >&2
    exit 2
  fi
  prefix=$2
elif [ "$#" -ne 0 ]; then
  echo "usage: uninstall.sh [--prefix PREFIX]" >&2
  exit 2
fi

case "$prefix" in
  /*) ;;
  *) echo "install prefix must be an absolute path: $prefix" >&2; exit 2 ;;
esac

rm -f -- \
  "$prefix/bin/toyoterm" \
  "$prefix/share/applications/toyoterm.desktop" \
  "$prefix/share/icons/hicolor/1024x1024/apps/toyoterm.png" \
  "$prefix/share/icons/hicolor/scalable/apps/toyoterm.svg" \
  "$prefix/lib/toyoterm/uninstall.sh"
rmdir "$prefix/lib/toyoterm" 2>/dev/null || true

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$prefix/share/applications" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$prefix/share/icons/hicolor" >/dev/null 2>&1 || true
fi

echo "uninstalled toyoterm from $prefix"
