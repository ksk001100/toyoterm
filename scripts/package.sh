#!/bin/sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repository_root"

./scripts/check-licenses.sh
cargo build --release --locked

version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
host_target=$(rustc -vV | sed -n 's/^host: //p')
target=${CARGO_BUILD_TARGET:-$host_target}
archive_name="toyoterm-$version-$target"
staging_root=$(mktemp -d)
trap 'rm -rf "$staging_root"' EXIT HUP INT TERM
staging_directory="$staging_root/$archive_name"

mkdir -p "$staging_directory/licenses"
if [ -n "${CARGO_BUILD_TARGET:-}" ]; then
  binary_path="target/$target/release/toyoterm"
else
  binary_path="target/release/toyoterm"
fi
cp "$binary_path" "$staging_directory/toyoterm"
cp THIRD_PARTY_NOTICES.md "$staging_directory/THIRD_PARTY_NOTICES.md"
cp vendor/mruby/LICENSE "$staging_directory/licenses/mruby-MIT.txt"
if [ -f LICENSE ]; then
  cp LICENSE "$staging_directory/LICENSE"
fi

mkdir -p dist
tar -C "$staging_root" -czf "dist/$archive_name.tar.gz" "$archive_name"
echo "created dist/$archive_name.tar.gz"
