#!/usr/bin/env bash
# Tries an unpacked download as a person gets it (decision 0064). FOLDER holds its programs (on
# macOS, GigaSculpt.app/Contents/MacOS): gigasculpt and gs each print VERSION for --version and
# their usage for --help, and a session gs starts there remeshes a small sphere with Quad
# Remesh's helper from the download, the only one it can find: no cache folder holds one and
# GS_QUADWILD names none. On Windows (Git Bash) gigasculpt is gigasculpt.com, the console twin a
# terminal runs. Runs on Linux, macOS (its bash 3.2) and Windows; scratch goes in $TMPDIR.
#
#   .github/release/try-download.sh VERSION FOLDER
set -euo pipefail
version=${1:?usage: try-download.sh VERSION FOLDER}
dir=$(cd "${2:?usage: try-download.sh VERSION FOLDER}" && pwd)
case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) app="$dir/gigasculpt.com" gs="$dir/gs.exe" windows=1 ;;
  *) app="$dir/gigasculpt" gs="$dir/gs" windows="" ;;
esac
# A path as the programs read it: on Windows, with its drive letter.
native() { if [[ -n $windows ]]; then cygpath -m "$1"; else echo "$1"; fi; }
fail() {
  echo "try-download.sh: $*" >&2
  exit 1
}

for program in "$app" "$gs"; do
  name=$(basename "$program")
  name=${name%.*}
  said=$("$program" --version)
  [[ $said == "$name $version" ]] || fail "$name --version says '$said', not '$name $version'"
  usage=$("$program" --help)
  [[ $usage == *"Usage: $name"* ]] || fail "$name --help shows no usage: $usage"
  echo "$name --version: $said; --help shows its usage"
done

scratch=$(mktemp -d "${TMPDIR:-/tmp}/try-download.XXXXXX")
mkdir "$scratch/cache"
XDG_CACHE_HOME=$(native "$scratch/cache")
export XDG_CACHE_HOME
unset GS_QUADWILD GS_ENGINE GS_SESSION
# Linux keeps sessions in $XDG_RUNTIME_DIR, which a desktop sets and a bare container does not.
if [[ $(uname -s) == Linux && -z ${XDG_RUNTIME_DIR:-} ]]; then
  mkdir -m 700 "$scratch/run"
  export XDG_RUNTIME_DIR="$scratch/run"
fi

session=try-download
# Each call says what it starts, so a run that stops shows where. gs spawn writes to a file, not
# a pipe: on Windows the engine it starts holds a copy of gs's output, so $(...) would wait
# until the engine ends (reported with decision 0064's trial runs).
echo "starting a session: gs spawn $session"
"$gs" spawn "$session" --wait-ms 60000 >"$scratch/spawn.txt"
cat "$scratch/spawn.txt"
key=$(sed -n 's/^stop key: \([^ ]*\).*/\1/p' "$scratch/spawn.txt")
[[ -n $key ]] || fail "gs spawn printed no stop key"
trap '"$gs" -s "$session" stop --key "$key" >/dev/null 2>&1 || true' EXIT
echo "making a sphere: gs call primitive.sphere"
"$gs" -s "$session" call primitive.sphere subdivisions=16
echo "remeshing it: gs call mesh.quad_remesh"
reply=$("$gs" -s "$session" --json call mesh.quad_remesh faces=400)
quads=$(printf '%s\n' "$reply" | grep -Eo '"quads": *[0-9]+' | grep -Eo '[0-9]+$' || true)
[[ ${quads:-0} -gt 0 ]] || fail "Quad Remesh made no quads: $reply"
sandboxed=$(printf '%s\n' "$reply" | grep -Eo '"sandboxed": *[a-z]+' | grep -Eo '[a-z]+$' || true)
echo "Quad Remesh: $quads quads for 400 (sandboxed: ${sandboxed:-?}), with the download's helper"
echo "stopping the session: gs stop"
"$gs" -s "$session" stop --key "$key"
trap - EXIT
echo "the download works: $dir"
