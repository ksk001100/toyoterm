#!/bin/sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repository_root"

required_files="
LICENSE
THIRD_PARTY_NOTICES.md
vendor/mruby/LICENSE
vendor/mruby/README.md
vendor/mruby/mruby.c
vendor/mruby/mruby.h
"

for required_file in $required_files; do
  if [ ! -s "$required_file" ]; then
    echo "license check: missing or empty $required_file" >&2
    exit 1
  fi
done

copyright='Copyright (c) 2010- mruby developers'
project_copyright='Copyright (c) 2026 Keisuke Toyota'
permission='The above copyright notice and this permission notice shall be included'

grep -Fq "$project_copyright" LICENSE
grep -Fq "$permission" LICENSE
grep -Fq "$copyright" vendor/mruby/LICENSE
grep -Fq "$copyright" THIRD_PARTY_NOTICES.md
grep -Fq "$permission" vendor/mruby/LICENSE
grep -Fq "$permission" THIRD_PARTY_NOTICES.md
grep -Fq 'mruby 4.0.0' vendor/mruby/README.md
grep -Fq '831da26b9021de0369d17b71b5667e2941a1a32d' vendor/mruby/README.md

echo "license check: project and vendored mruby notices are present"
