#!/bin/sh
set -eu

if [ "$#" -ne 1 ]; then
  echo "usage: windows-msi-version.sh CARGO_VERSION" >&2
  exit 2
fi

# Cargo supplies a SemVer version; MSI only accepts the numeric core.
core_version=$(printf '%s\n' "$1" | sed 's/[-+].*//')
if ! printf '%s\n' "$core_version" | awk -F. '
  NF != 3 { exit 1 }
  {
    for (i = 1; i <= 3; i++) {
      if ($i !~ /^[0-9]+$/ || (length($i) > 1 && substr($i, 1, 1) == "0")) exit 1
    }
    if ($1 > 255 || $2 > 255 || $3 > 65535) exit 1
  }
'; then
  echo "Cargo version $1 cannot be represented as an MSI version (major/minor: 0..255, patch: 0..65535)" >&2
  exit 1
fi

printf '%s\n' "$core_version"
