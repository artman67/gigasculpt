#!/usr/bin/env bash
# Assembles the pinned source of quadwild-bimdf (decision 0038) into DIR/src from
# sources.lock, and checks it. The only step that may use the network; nothing in it runs code
# from the source. build.sh builds it afterwards with the network off.
#
#   third_party/quadwild/fetch.sh DIR [--seed REPO]
#
# Each git line is unpacked with `git archive` from a git directory that already holds its
# commit: one under REPO (a clone of quadwild-bimdf with its submodules), else a fresh shallow
# fetch of that one commit from its URL. git checks every object it fetches against its hash,
# and the commit names the tree. Then every file of the assembled source is hashed and the list
# hashed again; that digest must match the lock's tree line.
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
mkdir -p "$src" "$work"

# The git directories under the seed (the clone's own and each submodule's).
gitdirs=()
if [[ -n "$seed" ]]; then
  while IFS= read -r d; do gitdirs+=("$d"); done < <(
    find "$seed/.git" -name HEAD -type f -not -path '*/logs/*' -printf '%h\n' | sort)
fi

# A git directory holding every object of COMMIT's tree, else nothing.
from_seed() {
  local d
  for d in "${gitdirs[@]}"; do
    if git --git-dir="$d" cat-file -e "$1^{tree}" 2>/dev/null &&
      git --git-dir="$d" archive "$1" >/dev/null 2>&1; then
      echo "$d"
      return
    fi
  done
}

n=0
while read -r kind a b c; do
  case "$kind" in
    git)
      path=$a commit=$b url=$c
      gd=$(from_seed "$commit")
      if [[ -z "$gd" ]]; then
        n=$((n + 1))
        gd="$work/git-$n"
        git init -q --bare "$gd"
        echo "fetching $path at $commit from $url"
        git --git-dir="$gd" fetch -q --depth 1 --no-tags "$url" "$commit"
      else
        echo "unpacking $path at $commit from the seed"
      fi
      [[ $(git --git-dir="$gd" rev-parse "$commit^{commit}") == "$commit" ]]
      mkdir -p "$src/$path"
      git --git-dir="$gd" archive "$commit" | tar -x -C "$src/$path"
      ;;
    tar)
      path=$a sum=$b url=$c
      file="$work/$(basename "$url")"
      echo "downloading $url"
      curl -fsSL --proto '=https' -o "$file" "$url"
      got=$(sha256sum "$file" | cut -d' ' -f1)
      if [[ $got != "$sum" ]]; then
        echo "fetch.sh: $url has SHA-256 $got, the lock says $sum" >&2
        exit 1
      fi
      mkdir -p "$src/$path"
      tar -x -f "$file" -C "$src/$path" --strip-components=1 --no-same-owner
      ;;
    tree)
      got=$("$here/digest.sh" "$src")
      if [[ $got != "$a" ]]; then
        echo "fetch.sh: the source's tree digest is $got, the lock says $a" >&2
        exit 1
      fi
      echo "tree digest $got matches"
      ;;
    *) ;;
  esac
done < <(grep -v '^#' "$lock")
echo "source ready in $src (the downloads stay in $work)"
