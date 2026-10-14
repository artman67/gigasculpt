#!/usr/bin/env bash
# The SHA-256 digest of a source tree (decision 0038): a line per file under DIR (its SHA-256
# and path) and per symbolic link (the SHA-256 of its target's name and its path), the lines
# sorted, and that list hashed again. File modes and times are not part of it.
#
#   third_party/quadwild/digest.sh DIR
set -euo pipefail
cd "${1:?usage: digest.sh DIR}"
{
  find . -type f -print0 | xargs -0 -r sha256sum
  find . -type l -print0 | while IFS= read -r -d '' f; do
    printf '%s  %s\n' "$(readlink "$f" | sha256sum | cut -d' ' -f1)" "$f"
  done
} | LC_ALL=C sort | sha256sum | cut -d' ' -f1
