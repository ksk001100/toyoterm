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

write_checksum() {
  artifact=$1
  checksum_path="$artifact.sha256"
  artifact_name=$(basename "$artifact")
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$(dirname "$artifact")" && sha256sum "$artifact_name") > "$checksum_path"
  else
    (cd "$(dirname "$artifact")" && shasum -a 256 "$artifact_name") > "$checksum_path"
  fi
}

mkdir -p dist
created_artifacts=
case "$target" in
  *-apple-darwin)
    app_directory="$staging_directory/toyoterm.app"
    mkdir -p "$app_directory/Contents/MacOS" "$app_directory/Contents/Resources"
    cp "$binary_directory/toyoterm" "$app_directory/Contents/MacOS/toyoterm"
    sed -e "s/@VERSION@/$version/g" packaging/macos/Info.plist \
      > "$app_directory/Contents/Info.plist"
    iconset_directory="$staging_root/toyoterm.iconset"
    mkdir -p "$iconset_directory"
    make_icon() {
      pixels=$1
      filename=$2
      sips -z "$pixels" "$pixels" packaging/app-icon.png \
        --out "$iconset_directory/$filename" >/dev/null
    }
    make_icon 16 icon_16x16.png
    make_icon 32 icon_16x16@2x.png
    make_icon 32 icon_32x32.png
    make_icon 64 icon_32x32@2x.png
    make_icon 128 icon_128x128.png
    make_icon 256 icon_128x128@2x.png
    make_icon 256 icon_256x256.png
    make_icon 512 icon_256x256@2x.png
    make_icon 512 icon_512x512.png
    make_icon 1024 icon_512x512@2x.png
    iconutil -c icns "$iconset_directory" \
      -o "$app_directory/Contents/Resources/toyoterm.icns"
    copy_common_files "$app_directory/Contents/Resources"
    plutil -lint "$app_directory/Contents/Info.plist"
    archive_path="dist/$archive_name.tar.gz"
    tar -C "$staging_root" -czf "$archive_path" "$archive_name"
    created_artifacts=$archive_path
    if command -v hdiutil >/dev/null 2>&1; then
      dmg_directory="$staging_root/dmg"
      mkdir -p "$dmg_directory"
      cp -R "$app_directory" "$dmg_directory/toyoterm.app"
      ln -s /Applications "$dmg_directory/Applications"
      dmg_path="dist/$archive_name.dmg"
      hdiutil create -quiet -ov -format UDZO -volname "toyoterm $version" \
        -srcfolder "$dmg_directory" "$dmg_path"
      hdiutil verify -quiet "$dmg_path"
      created_artifacts="$created_artifacts $dmg_path"
    fi
    ;;
  *-windows-*)
    copy_common_files "$staging_directory"
    cp "$binary_directory/toyoterm.exe" "$staging_directory/toyoterm.exe"
    cp packaging/windows/Install-Toyoterm.ps1 packaging/windows/Uninstall-Toyoterm.ps1 \
      "$staging_directory/"
    archive_path="dist/$archive_name.zip"
    if command -v powershell.exe >/dev/null 2>&1; then
      windows_staging=$(cygpath -w "$staging_directory")
      windows_archive=$(cygpath -w "$repository_root/$archive_path")
      powershell.exe -NoProfile -Command \
        "Compress-Archive -Force -Path '$windows_staging' -DestinationPath '$windows_archive'"
    else
      tar -C "$staging_root" -a -cf "$archive_path" "$archive_name"
    fi
    created_artifacts=$archive_path
    ;;
  *-linux-*)
    copy_common_files "$staging_directory"
    cp "$binary_directory/toyoterm" "$staging_directory/toyoterm"
    cp packaging/linux/install.sh packaging/linux/uninstall.sh "$staging_directory/"
    chmod 755 "$staging_directory/install.sh" "$staging_directory/uninstall.sh"
    mkdir -p \
      "$staging_directory/share/applications" \
      "$staging_directory/share/icons/hicolor/1024x1024/apps"
    cp packaging/linux/toyoterm.desktop "$staging_directory/share/applications/"
    cp packaging/app-icon.png \
      "$staging_directory/share/icons/hicolor/1024x1024/apps/toyoterm.png"
    archive_path="dist/$archive_name.tar.gz"
    tar -C "$staging_root" -czf "$archive_path" "$archive_name"
    created_artifacts=$archive_path
    ;;
  *)
    echo "unsupported release target: $target" >&2
    exit 1
    ;;
esac

sh scripts/verify-package.sh "$archive_path" "$version" "$target"
for artifact in $created_artifacts; do
  write_checksum "$artifact"
  echo "created $artifact"
  echo "created $artifact.sha256"
done
