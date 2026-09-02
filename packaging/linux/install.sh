#!/bin/sh
set -eu

prefix=${TOYOTERM_PREFIX:-"${HOME:?HOME is not set}/.local"}
if [ "${1:-}" = "--prefix" ]; then
  if [ "$#" -ne 2 ] || [ -z "$2" ]; then
    echo "usage: ./install.sh [--prefix PREFIX]" >&2
    exit 2
  fi
  prefix=$2
elif [ "$#" -ne 0 ]; then
  echo "usage: ./install.sh [--prefix PREFIX]" >&2
  exit 2
fi

case "$prefix" in
  /*) ;;
  *) echo "install prefix must be an absolute path: $prefix" >&2; exit 2 ;;
esac

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
binary_directory="$prefix/bin"
application_directory="$prefix/share/applications"
icon_directory="$prefix/share/icons/hicolor/scalable/apps"
support_directory="$prefix/lib/toyoterm"

mkdir -p "$binary_directory" "$application_directory" "$icon_directory" "$support_directory"
temporary_binary="$binary_directory/.toyoterm-install-$$"
temporary_desktop="$application_directory/.toyoterm.desktop-$$"
trap 'rm -f "$temporary_binary" "$temporary_desktop"' EXIT HUP INT TERM
install -m 755 "$script_directory/toyoterm" "$temporary_binary"
mv -f "$temporary_binary" "$binary_directory/toyoterm"
install -m 644 "$script_directory/share/icons/hicolor/scalable/apps/toyoterm.svg" \
  "$icon_directory/toyoterm.svg"
install -m 755 "$script_directory/uninstall.sh" "$support_directory/uninstall.sh"

escaped_binary=$(printf '%s' "$binary_directory/toyoterm" \
  | sed 's/\\/\\\\/g; s/"/\\"/g; s/`/\\`/g; s/\$/\\$/g')
while IFS= read -r desktop_line; do
  case "$desktop_line" in
    'Exec="@TOYOTERM_BINARY@"') printf 'Exec="%s"\n' "$escaped_binary" ;;
    *) printf '%s\n' "$desktop_line" ;;
  esac
done < "$script_directory/share/applications/toyoterm.desktop" > "$temporary_desktop"
chmod 644 "$temporary_desktop"
mv -f "$temporary_desktop" "$application_directory/toyoterm.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$application_directory" >/dev/null 2>&1 || true
fi

echo "installed toyoterm to $binary_directory/toyoterm"
case ":${PATH:-}:" in
  *":$binary_directory:"*) ;;
  *) echo "add $binary_directory to PATH to use toyoterm from a shell" ;;
esac
echo "uninstall with $support_directory/uninstall.sh"
