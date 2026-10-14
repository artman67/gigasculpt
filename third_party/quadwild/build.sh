#!/usr/bin/env bash
# Builds the Quad Remesh engine (decision 0038) from DIR/src, which fetch.sh assembled, into
# DIR/program: the two programs, their configs, every licence text and a stamp naming the pin.
# On Linux the whole build runs inside bubblewrap with the network off, the system read-only,
# the source read-only, and only DIR writable; on macOS and Windows (Git Bash, on the release
# and CI machines) there is no bubblewrap and it runs as it is, CMake still told to download
# nothing. Every non-free solver is configured out, and the finished programs are checked for
# them before the stamp is written; a program folder without its stamp is never used. macOS
# gets one program for both Apple silicon and Intel.
#
#   third_party/quadwild/build.sh DIR [JOBS]     (JOBS defaults to 6; run it through gate/slot.sh)
#
# DIR is usually ${XDG_CACHE_HOME:-~/.cache}/gigasculpt/quadwild/PIN, where the engine looks.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
dir=$(cd "${1:?usage: build.sh DIR [JOBS]}" && pwd)
jobs=${2:-6}
pin=$(awk '$1 == "pin" { print $2 }' "$here/sources.lock")
src="$dir/src" build="$dir/build" out="$dir/program"
case "$(uname -s)" in
  Linux) system=linux exe="" ;;
  Darwin) system=macos exe="" ;;
  MINGW* | MSYS* | CYGWIN*) system=windows exe=.exe ;;
  *) echo "build.sh: no build for $(uname -s)" >&2; exit 1 ;;
esac

if [[ -z ${QUADWILD_SANDBOXED:-} ]]; then
  [[ -d $src ]] || { echo "build.sh: no source in $src; run fetch.sh first" >&2; exit 1; }
  for d in "$out" "$out.part"; do
    if [[ -e $d ]]; then echo "build.sh: $d exists; remove it first" >&2; exit 1; fi
  done
  mkdir -p "$build" "$dir/home" "$dir/tmp"
  if [[ $system == linux ]]; then
    exec bwrap --ro-bind / / --dev /dev --proc /proc --tmpfs /tmp \
      --bind "$dir" "$dir" --ro-bind "$src" "$src" \
      --unshare-net --unshare-pid --unshare-ipc --unshare-uts --die-with-parent \
      --setenv HOME "$dir/home" --setenv TMPDIR "$dir/tmp" --setenv QUADWILD_SANDBOXED 1 \
      --chdir "$dir" "$here/build.sh" "$dir" "$jobs"
  fi
fi

# A path as CMake, a Windows program there, reads it.
native() { if [[ $system == windows ]]; then cygpath -m "$1"; else echo "$1"; fi; }

# Recommendation 2 of research 20: Blossom V, Gurobi, CPLEX, Mosek and the other solvers out;
# no OpenMP (one thread, decision 0038 section 2). CoMISo still links BLAS and LAPACK when the
# build machine has them (decision 0038, amended 2026-10-14), so the check below prints what
# each program links.
flags=(
  -DCMAKE_BUILD_TYPE=Release
  -DSATSUMA_ENABLE_BLOSSOM5=OFF
  -DCMAKE_DISABLE_FIND_PACKAGE_Gurobi=ON -DQUADRETOPOLOGY_WITH_GUROBI=OFF
  -DLEMON_ENABLE_GLPK=NO -DLEMON_ENABLE_ILOG=NO -DLEMON_ENABLE_COIN=NO -DLEMON_ENABLE_SOPLEX=NO
  -DCOMISO_ENABLE_GUROBI=OFF -DCOMISO_ENABLE_CPLEX=OFF -DCOMISO_ENABLE_MOSEK=OFF
  -DCOMISO_ENABLE_OPENMP=OFF
  -DGMM_INCLUDE_DIR="$(native "$src/gmm/include")"
  -DFETCHCONTENT_FULLY_DISCONNECTED=ON
)
# Each system's compiler as quadwild-bimdf's own release builds use it: Ninja on Linux; on
# macOS both architectures in one program; on Windows Visual Studio's clang-cl.
case $system in
  linux) flags=(-G Ninja "${flags[@]}") ;;
  macos) flags+=(-DCMAKE_OSX_ARCHITECTURES="arm64;x86_64") ;;
  windows) flags+=(-T ClangCl) ;;
esac
cmake -S "$(native "$src")" -B "$(native "$build")" "${flags[@]}"
cmake --build "$(native "$build")" --config Release -j "$jobs"

# Visual Studio puts each configuration's programs in a folder of its own.
bin="$build/Build/bin"
if [[ -d $bin/Release ]]; then bin="$bin/Release"; fi
stage="$out.part"
mkdir -p "$stage/LICENSES"
for p in quadwild quad_from_patches; do cp "$bin/$p$exe" "$stage/"; done
cp -r "$src/config" "$stage/config"
(cd "$src" && find . -maxdepth 3 -type f \( -iname 'licen[cs]e*' -o -iname 'copying*' \) \
  -not -path './libs/glew/*' | sed 's|^\./||') | while IFS= read -r f; do
  mkdir -p "$stage/LICENSES/$(dirname "$f")"
  cp "$src/$f" "$stage/LICENSES/$f"
done
mkdir -p "$stage/LICENSES/libs/lpsolve"
cp "$here/LICENSE-lpsolve-LGPL-2.1.txt" "$stage/LICENSES/libs/lpsolve/LGPL-2.1.txt"
cp "$here/sources.lock" "$stage/sources.lock"

# No non-free solver anywhere: no such library linked, no such symbol compiled in. Blossom V's
# class is a bare `PerfectMatching`; LEMON's own `lemon::MaxWeightedPerfectMatching` (Edmonds'
# blossom algorithm, Boost licence) is free and stays. A Windows program keeps its symbols in a
# separate file, so there only the libraries it loads are checked.
libs='gurobi|cplex|mosek|glpk|soplex|libcoin|libclp|libcbc|libosi|blossom'
syms='(^| )PerfectMatching::|[Bb]lossom5|\bGRB[a-z]|GUROBISolver|\bCPX[A-Za-z]|CPLEXSolver|\bMSK_|MOSEKSolver|\bglp_|soplex::|ClpSimplex|CbcModel|OsiSolverInterface'
# The libraries a program loads, one a line, the name or path first.
linked() {
  case $system in
    linux) ldd "$1" ;;
    macos) otool -L "$1" | tail -n +2 ;;
    windows) grep -a -o -i -E '[a-z0-9_.+-]+\.dll' "$1" | sort -u ;;
  esac
}
symbols() { if [[ $system == linux ]]; then nm -C "$1"; else nm "$1" | c++filt; fi; }
for p in quadwild quad_from_patches; do
  program="$stage/$p$exe"
  if linked "$program" | grep -Ei "$libs"; then
    echo "build.sh: $p links a non-free solver" >&2
    exit 1
  fi
  links=$(linked "$program" | awk '{print $1}' | tr '\n' ' ')
  if [[ $system == windows ]]; then
    echo "$p: links $links(its symbols are not in the program to check)"
    continue
  fi
  total=$(($(symbols "$program" | wc -l)))
  if ((total < 1000)); then echo "build.sh: $p has no symbols to check ($total)" >&2; exit 1; fi
  if symbols "$program" | grep -E "$syms"; then
    echo "build.sh: $p holds a non-free solver" >&2
    exit 1
  fi
  echo "$p: $total symbols, none from a non-free solver; links $links"
done
echo "$pin" >"$stage/stamp"
mv "$stage" "$out"
echo "program $pin ready in $out ($(du -sh "$out" | cut -f1))"
