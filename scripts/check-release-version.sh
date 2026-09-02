#!/bin/sh
set -eu

if [ "$#" -ne 1 ]; then
  echo "usage: check-release-version.sh vVERSION" >&2
  exit 2
fi

tag=$1
case "$tag" in
  v*) tag_version=${tag#v} ;;
  *) echo "release tag must start with v: $tag" >&2; exit 1 ;;
esac

package_version=$(cargo pkgid --locked -p toyoterm-cli | sed 's/.*[#@]//')
if [ "$tag_version" != "$package_version" ]; then
  echo "release tag $tag does not match Cargo version $package_version" >&2
  exit 1
fi

reported_version=$(cargo run --quiet --locked -- version)
if [ "$reported_version" != "toyoterm $package_version" ]; then
  echo "binary version does not match Cargo version: $reported_version" >&2
  exit 1
fi

echo "release version check: $tag matches Cargo and the binary"

