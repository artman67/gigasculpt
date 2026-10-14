#!/usr/bin/env bash
# The SHA-256 digest of a source tree (decision 0038): a line per file under DIR (its SHA-256
# and path) and per symbolic link (the SHA-256 of its target's name and its path), the lines
# sorted, and that list hashed again. File modes and times are not part of it. The same on
# Linux, macOS (which has shasum where Linux has sha256sum) and Windows' Git Bash.
#
#   third_party/quadwild/digest.sh DIR
set -euo pipefail
cd "${1:?usage: digest.sh DIR}"
# Twin of fetch.sh's: the one SHA-256 tool each system has.
if command -v sha256sum >/dev/null; then sha256=(sha256sum); else sha256=(shasum -a 256); fi
# Linux's and macOS's tools print "HASH  NAME". Git Bash's sha256sum reads in binary mode unless
# told otherwise and marks it, "HASH *NAME"; sed takes the mark out, so every system lists alike.
{
  find . -type f -exec "${sha256[@]}" {} + | sed 's/^\([0-9a-f]\{64\}\) \*/\1  /'
  find . -type l -print0 | while IFS= read -r -d '' f; do
    printf '%s  %s\n' "$(readlink "$f" | "${sha256[@]}" | cut -d' ' -f1)" "$f"
  done
} | LC_ALL=C sort | "${sha256[@]}" | cut -d' ' -f1
