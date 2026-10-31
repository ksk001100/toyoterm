#!/bin/sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repository_root"

required_files="
LICENSE
THIRD_PARTY_NOTICES.md
vendor/conpty/LICENSE
vendor/conpty/README.md
vendor/conpty/win-x64/conpty.dll
vendor/conpty/win-x64/OpenConsole.exe
packaging/windows/LICENSE.rtf
"

for required_file in $required_files; do
  if [ ! -s "$required_file" ]; then
    echo "license check: missing or empty $required_file" >&2
    exit 1
  fi
done

project_copyright='Copyright (c) 2026 Keisuke Toyota'
permission='The above copyright notice and this permission notice shall be included'

grep -Fq "$project_copyright" LICENSE
grep -Fq "$permission" LICENSE
grep -Fq "$project_copyright" packaging/windows/LICENSE.rtf
grep -Fq "$permission" packaging/windows/LICENSE.rtf
grep -Fq '## Steel ' THIRD_PARTY_NOTICES.md
grep -Fq 'MIT OR Apache-2.0' THIRD_PARTY_NOTICES.md
grep -Fq 'Microsoft.Windows.Console.ConPTY 1.25.260710002-preview' vendor/conpty/README.md
grep -Fq 'Copyright (c) Microsoft Corporation' vendor/conpty/LICENSE
grep -Fq 'Microsoft Windows Console ConPTY' THIRD_PARTY_NOTICES.md

file_hash() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{ print toupper($1) }'
  else
    shasum -a 256 "$1" | awk '{ print toupper($1) }'
  fi
}

test "$(file_hash vendor/conpty/win-x64/conpty.dll)" = \
  'E2FE87E2258C4E46FFC5157F727218CC25F34A174902F72EB8A5B49EDD9A6458'
test "$(file_hash vendor/conpty/win-x64/OpenConsole.exe)" = \
  '2525C351AA136D555E5DF9A3C9D6CE9BE43F785E37E3C993B8F23B3F0A53C7FA'

echo "license check: project, Steel, and ConPTY notices and binaries are verified"
