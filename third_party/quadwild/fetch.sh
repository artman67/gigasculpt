#!/usr/bin/env bash
# Assembles the pinned source of quadwild-bimdf (decision 0038) into DIR/src from
# sources.lock, and checks it. The only step that may use the network; nothing in it runs code
# from the source. build.sh builds it afterwards with the network off.
#
#   third_party/quadwild/fetch.sh DIR [--seed REPO]
#
# Two sources, in this order (decision 0038, amended 2026-10-14 (Release-prep)):
# 1. The archive: the lock's archive line names the project's own copy of the assembled source,
#    one download whose SHA-256 is checked before it is unpacked.
# 2. The lock's lines, each from its own home. Each git line is unpacked with `git archive`
#    from a git directory that already holds its commit: one under REPO (a clone of
#    quadwild-bimdf with its submodules), else a fresh shallow fetch of that one commit from its
#    URL. git checks every object it fetches against its hash, and the commit names the tree.
#    A tar line's tarball must have the lock's SHA-256.
# Each source is assembled in DIR/fetch; then every file of it is hashed and the list hashed
# again, and only when that digest matches the lock's tree line is it moved to DIR/src. A source
# that cannot be had or does not check out is passed over, saying why; when neither does,
# fetch.sh refuses. Runs on Linux, macOS (its bash 3.2 and BSD tools too) and Windows' Git Bash,
# and unpacks the same bytes on each: git writes no CRLF line endings here, whatever the
# machine's git settings.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
lock="$here/sources.lock"
dir=${1:?usage: fetch.sh DIR [--seed REPO]}
seed=""
if [[ ${2:-} == --seed ]]; then seed=$(cd "${3:?--seed needs a path}" && pwd); fi
mkdir -p "$dir"
dir=$(cd "$dir" && pwd)
src="$dir/src"
if [[ -e "$src" ]]; then
  echo "fetch.sh: $src exists; remove it first, the source is unpacked into an empty folder" >&2
  exit 1
fi
work="$dir/fetch"
if [[ -e "$work" ]]; then
  echo "fetch.sh: $work exists; remove it first" >&2
  exit 1
fi
mkdir -p "$work"
tree=$(awk '$1 == "tree" { print $2 }' "$lock")
archive=$(awk '$1 == "archive" { print $2, $3 }' "$lock")

# The git directories under the seed (the clone's own and each submodule's).
gitdirs=()
if [[ -n "$seed" ]]; then
  while IFS= read -r d; do gitdirs+=("$d"); done < <(
    find "$seed/.git" -name HEAD -type f -not -path '*/logs/*' | sed 's|/HEAD$||' | sort)
fi

# git as fetch.sh runs it: no line ending changes, so Windows unpacks what Linux does.
git() { command git -c core.autocrlf=false -c core.eol=lf "$@"; }

# Twin of digest.sh's: the one SHA-256 tool each system has; only its hash (field 1) is read.
if command -v sha256sum >/dev/null; then sha256=(sha256sum); else sha256=(shasum -a 256); fi

# The functions below run where bash ignores `set -e` (left of ||), so each step that can fail
# returns by itself.

# A git directory holding every object of COMMIT's tree, else nothing.
from_seed() {
  local d
  # Written so for an empty list too, which bash 3.2 would call unbound.
  for d in ${gitdirs[@]+"${gitdirs[@]}"}; do
    if git --git-dir="$d" cat-file -e "$1^{tree}" 2>/dev/null &&
      git --git-dir="$d" archive "$1" >/dev/null 2>&1; then
      echo "$d"
      return
    fi
  done
}

# Downloads URL into FILE and checks that its SHA-256 is SUM.
download() {
  local url=$1 file=$2 sum=$3 got
  echo "downloading $url"
  curl -fsSL --proto '=https,file' -o "$file" "$url" || return 1
  got=$("${sha256[@]}" "$file" | cut -d' ' -f1)
  if [[ $got != "$sum" ]]; then
    echo "fetch.sh: $url has SHA-256 $got, the lock says $sum" >&2
    return 1
  fi
}

# Unpacks the tarball FILE into the folder TO, without its top folder.
untar() { mkdir -p "$2" && tar -x -f "$1" -C "$2" --strip-components=1 --no-same-owner; }

# Source 1 into TO: the archive.
from_archive() {
  local sum=${archive%% *} url=${archive#* }
  if [[ -z $archive ]]; then
    echo "fetch.sh: the lock has no archive line" >&2
    return 1
  fi
  download "$url" "$work/$(basename "$url")" "$sum" && untar "$work/$(basename "$url")" "$1"
}

# Source 2 into TO: each line from its home (or the seed).
from_lines() {
  local kind a b c gd n=0
  while read -r kind a b c; do
    case "$kind" in
      git)
        gd=$(from_seed "$b")
        if [[ -z "$gd" ]]; then
          n=$((n + 1))
          gd="$work/git-$n"
          git init -q --bare "$gd" || return 1
          echo "fetching $a at $b from $c"
          git --git-dir="$gd" fetch -q --depth 1 --no-tags "$c" "$b" || return 1
        else
          echo "unpacking $a at $b from the seed"
        fi
        [[ $(git --git-dir="$gd" rev-parse "$b^{commit}") == "$b" ]] || return 1
        mkdir -p "$1/$a" && git --git-dir="$gd" archive "$b" | tar -x -C "$1/$a" || return 1
        ;;
      tar)
        download "$c" "$work/$(basename "$c")" "$b" && untar "$work/$(basename "$c")" "$1/$a" ||
          return 1
        ;;
      *) ;;
    esac
  done < <(grep -v '^#' "$lock")
}

# Assembles source NAME (archive or lines) and moves it to DIR/src if its tree is the lock's.
take() {
  local got at="$work/$1"
  "from_$1" "$at" || return 1
  got=$("$here/digest.sh" "$at") || return 1
  if [[ $got != "$tree" ]]; then
    echo "fetch.sh: the $1's tree digest is $got, the lock says $tree" >&2
    return 1
  fi
  echo "tree digest $got matches"
  mv "$at" "$src"
}

if take archive; then
  from="the archive"
else
  echo "fetch.sh: passed over the archive; taking each line from its home" >&2
  if take lines; then
    from="the lock's lines"
  else
    echo "fetch.sh: refused: neither the archive nor the lock's lines gave the lock's tree" >&2
    exit 1
  fi
fi
echo "source ready in $src, from $from (the downloads stay in $work)"
