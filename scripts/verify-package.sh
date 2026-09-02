#!/bin/sh
set -eu

if [ "$#" -ne 3 ]; then
  echo "usage: verify-package.sh ARCHIVE VERSION TARGET" >&2
  exit 2
fi

archive=$1
version=$2
target=$3
archive_name="toyoterm-$version-$target"

if [ ! -s "$archive" ]; then
  echo "package verification: missing or empty archive: $archive" >&2
  exit 1
fi

list_archive() {
  case "$target" in
    *-windows-*)
      if command -v powershell.exe >/dev/null 2>&1; then
        archive_directory=$(CDPATH= cd -- "$(dirname -- "$archive")" && pwd)
        windows_archive=$(cygpath -w "$archive_directory/$(basename "$archive")")
        TOYOTERM_VERIFY_ARCHIVE="$windows_archive" powershell.exe -NoProfile -Command '
          Add-Type -AssemblyName System.IO.Compression.FileSystem
          $zip = [IO.Compression.ZipFile]::OpenRead($env:TOYOTERM_VERIFY_ARCHIVE)
          try { $zip.Entries | ForEach-Object { $_.FullName } }
          finally { $zip.Dispose() }
        '
      elif command -v bsdtar >/dev/null 2>&1; then
        bsdtar -tf "$archive"
      else
        tar -tf "$archive"
      fi
      ;;
    *)
      tar -tf "$archive"
      ;;
  esac
}

extract_archive() {
  destination=$1
  case "$target" in
    *-windows-*)
      if command -v powershell.exe >/dev/null 2>&1; then
        archive_directory=$(CDPATH= cd -- "$(dirname -- "$archive")" && pwd)
        windows_archive=$(cygpath -w "$archive_directory/$(basename "$archive")")
        windows_destination=$(cygpath -w "$destination")
        TOYOTERM_VERIFY_ARCHIVE="$windows_archive" \
          TOYOTERM_VERIFY_DESTINATION="$windows_destination" \
          powershell.exe -NoProfile -Command '
            Expand-Archive -LiteralPath $env:TOYOTERM_VERIFY_ARCHIVE -DestinationPath $env:TOYOTERM_VERIFY_DESTINATION
          '
      elif command -v bsdtar >/dev/null 2>&1; then
        bsdtar -xf "$archive" -C "$destination"
      else
        tar -xf "$archive" -C "$destination"
      fi
      ;;
    *)
      tar -xf "$archive" -C "$destination"
      ;;
  esac
}

listing=$(list_archive | tr -d '\r' | tr '\\' '/')
if printf '%s\n' "$listing" | grep -Eq '(^/|(^|/)\.\.(/|$))'; then
  echo "package verification: archive contains an unsafe path" >&2
  exit 1
fi

require_entry() {
  entry=$1
  if ! printf '%s\n' "$listing" | grep -Fqx "$entry"; then
    echo "package verification: missing $entry" >&2
    exit 1
  fi
}

case "$target" in
  *-apple-darwin)
    common_prefix="$archive_name/toyoterm.app/Contents/Resources"
    executable="$archive_name/toyoterm.app/Contents/MacOS/toyoterm"
    require_entry "$archive_name/toyoterm.app/Contents/Info.plist"
    require_entry "$archive_name/toyoterm.app/Contents/Resources/toyoterm.icns"
    ;;
  *-windows-*)
    common_prefix=$archive_name
    executable="$archive_name/toyoterm.exe"
    require_entry "$archive_name/Install-Toyoterm.ps1"
    require_entry "$archive_name/Uninstall-Toyoterm.ps1"
    ;;
  *-linux-*)
    common_prefix=$archive_name
    executable="$archive_name/toyoterm"
    require_entry "$archive_name/install.sh"
    require_entry "$archive_name/uninstall.sh"
    require_entry "$archive_name/share/applications/toyoterm.desktop"
    require_entry "$archive_name/share/icons/hicolor/1024x1024/apps/toyoterm.png"
    ;;
  *)
    echo "package verification: unsupported target: $target" >&2
    exit 1
    ;;
esac
require_entry "$common_prefix/README.md"
require_entry "$common_prefix/README.ja.md"
require_entry "$common_prefix/LICENSE"
require_entry "$common_prefix/THIRD_PARTY_NOTICES.md"
require_entry "$common_prefix/examples/minimal_config.rb"
require_entry "$common_prefix/licenses/mruby-MIT.txt"
require_entry "$executable"

verification_root=$(mktemp -d)
trap 'rm -rf "$verification_root"' EXIT HUP INT TERM
extract_archive "$verification_root"
actual_version=$("$verification_root/$executable" version)
if [ "$actual_version" != "toyoterm $version" ]; then
  echo "package verification: expected 'toyoterm $version', got '$actual_version'" >&2
  exit 1
fi

if printf '%s' "$target" | grep -q -- '-linux-'; then
  install_prefix="$verification_root/install prefix"
  "$verification_root/$archive_name/install.sh" --prefix "$install_prefix" >/dev/null
  "$verification_root/$archive_name/install.sh" --prefix "$install_prefix" >/dev/null
  installed_version=$("$install_prefix/bin/toyoterm" version)
  if [ "$installed_version" != "toyoterm $version" ]; then
    echo "package verification: installed binary reported '$installed_version'" >&2
    exit 1
  fi
  if grep -Fq '@TOYOTERM_BINARY@' "$install_prefix/share/applications/toyoterm.desktop"; then
    echo "package verification: desktop entry still contains its placeholder" >&2
    exit 1
  fi
  if command -v desktop-file-validate >/dev/null 2>&1; then
    desktop-file-validate "$install_prefix/share/applications/toyoterm.desktop"
  fi
  "$install_prefix/lib/toyoterm/uninstall.sh" --prefix "$install_prefix" >/dev/null
  for installed_path in \
    "$install_prefix/bin/toyoterm" \
    "$install_prefix/share/applications/toyoterm.desktop" \
    "$install_prefix/share/icons/hicolor/1024x1024/apps/toyoterm.png" \
    "$install_prefix/share/icons/hicolor/scalable/apps/toyoterm.svg" \
    "$install_prefix/lib/toyoterm/uninstall.sh"; do
    if [ -e "$installed_path" ]; then
      echo "package verification: uninstaller left $installed_path behind" >&2
      exit 1
    fi
  done
fi

echo "package verification: $archive is complete and runnable"
