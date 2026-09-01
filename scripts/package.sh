#!/bin/sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repository_root"

./scripts/check-licenses.sh
cargo build --release --locked

version=$(cargo pkgid --locked -p toyoterm-cli | sed 's/.*[#@]//')
host_target=$(rustc -vV | sed -n 's/^host: //p')
target=${CARGO_BUILD_TARGET:-$host_target}
archive_name="toyoterm-$version-$target"
staging_root=$(mktemp -d)
trap 'rm -rf "$staging_root"' EXIT HUP INT TERM
staging_directory="$staging_root/$archive_name"

if [ -n "${CARGO_BUILD_TARGET:-}" ]; then
  binary_directory="target/$target/release"
else
  binary_directory="target/release"
fi

copy_common_files() {
  destination=$1
  mkdir -p "$destination/licenses" "$destination/examples"
  cp LICENSE "$destination/LICENSE"
  cp THIRD_PARTY_NOTICES.md "$destination/THIRD_PARTY_NOTICES.md"
  cp vendor/mruby/LICENSE "$destination/licenses/mruby-MIT.txt"
  cp README.md README.ja.md "$destination/"
  cp examples/minimal_config.rb "$destination/examples/minimal_config.rb"
}

mkdir -p dist
case "$target" in
  *-apple-darwin)
    app_directory="$staging_directory/toyoterm.app"
    mkdir -p "$app_directory/Contents/MacOS" "$app_directory/Contents/Resources"
    cp "$binary_directory/toyoterm" "$app_directory/Contents/MacOS/toyoterm"
    sed -e "s/@VERSION@/$version/g" packaging/macos/Info.plist \
      > "$app_directory/Contents/Info.plist"
    copy_common_files "$app_directory/Contents/Resources"
    archive_path="dist/$archive_name.tar.gz"
    tar -C "$staging_root" -czf "$archive_path" "$archive_name"
    ;;
  *-windows-*)
    copy_common_files "$staging_directory"
    cp "$binary_directory/toyoterm.exe" "$staging_directory/toyoterm.exe"
    archive_path="dist/$archive_name.zip"
    if command -v powershell.exe >/dev/null 2>&1; then
      windows_staging=$(cygpath -w "$staging_directory")
      windows_archive=$(cygpath -w "$repository_root/$archive_path")
      powershell.exe -NoProfile -Command \
        "Compress-Archive -Force -Path '$windows_staging' -DestinationPath '$windows_archive'"
    else
      tar -C "$staging_root" -a -cf "$archive_path" "$archive_name"
    fi
    ;;
  *-linux-*)
    copy_common_files "$staging_directory"
    cp "$binary_directory/toyoterm" "$staging_directory/toyoterm"
    archive_path="dist/$archive_name.tar.gz"
    tar -C "$staging_root" -czf "$archive_path" "$archive_name"
    ;;
  *)
    echo "unsupported release target: $target" >&2
    exit 1
    ;;
esac

echo "created $archive_path"
