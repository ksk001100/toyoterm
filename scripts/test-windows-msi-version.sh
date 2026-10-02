#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

assert_version() {
  actual=$(sh "$script_directory/windows-msi-version.sh" "$1")
  if [ "$actual" != "$2" ]; then
    echo "$1: expected $2, got $actual" >&2
    exit 1
  fi
}

assert_version 0.2.0-dev 0.2.0
assert_version 0.2.0-rc.1+build.42 0.2.0
assert_version 1.2.3+build.42 1.2.3
assert_version 1.2.3 1.2.3
assert_version 0.0.0 0.0.0
assert_version 255.255.65535 255.255.65535

for invalid in 256.0.0 0.256.0 0.0.65536 999999999999999999999.0.0 1.2 1.2.3.4 1..3 a.2.3 01.2.3 ''; do
  if sh "$script_directory/windows-msi-version.sh" "$invalid" >/dev/null 2>&1; then
    echo "expected MSI version rejection: $invalid" >&2
    exit 1
  fi
done

echo "Windows MSI version tests passed"
