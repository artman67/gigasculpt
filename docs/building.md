# Building from source

GigaSculpt is a Rust workspace. The window uses SDL3, which the build compiles from source and
links in, so you need CMake and a C compiler beside Rust. Everything else, the brushes,
materials, layouts and pictures included, is built into the two programs.

## What you need

On every system: Rust through [rustup](https://rustup.rs). The repository's
`rust-toolchain.toml` names the Rust version, and rustup installs it with clippy and rustfmt
the first time you build.

**Linux (Debian, Ubuntu):**

```
sudo apt install build-essential cmake pkg-config \
  libx11-dev libxext-dev libxrandr-dev libxcursor-dev libxfixes-dev libxi-dev libxss-dev \
  libxtst-dev libxkbcommon-dev libwayland-dev libdecor-0-dev libegl1-mesa-dev \
  libgl1-mesa-dev libdrm-dev libgbm-dev libdbus-1-dev libudev-dev
```

On other distributions, install the development packages of the same libraries. To run
GigaSculpt you need a Vulkan driver: Mesa's for AMD and Intel cards, or your card maker's.

**Windows:** Visual Studio 2022 or its Build Tools with "Desktop development with C++" (the
MSVC compiler and the Windows SDK), and CMake (Visual Studio's own, or from cmake.org).

**macOS:** the Xcode command line tools (`xcode-select --install`) and CMake (for example
`brew install cmake`).

GigaSculpt is made and tested on Linux. The Windows and macOS builds are new and less tested, so
please [report](troubleshooting.md#reporting-a-bug) what goes wrong there.

## Build and run

```
cargo build --release --locked
```

This makes two programs in `target/release/`:

- `gigasculpt`, the app. With no arguments it opens the window; `gigasculpt serve --headless`
  runs a session with no window. On Windows a release build opens no console, so it prints
  nothing in a terminal. A download also holds `gigasculpt.com`, the same program built for the
  console, which a terminal runs when you type `gigasculpt` (see
  [Scripts and agents](scripting.md#sessions-without-a-window)). To build it, build with
  `--features gigasculpt/console`, copy `target/release/gigasculpt.exe` to `gigasculpt.com`
  beside the other, then build again without the feature.
- `gs`, the command-line client and MCP server (see [Scripts and agents](scripting.md)). It
  looks for `gigasculpt` beside itself, so keep the two together.

`cargo build --locked` without `--release` builds faster and still runs well, as the
dependencies are optimized in both.

## Tests and lints

```
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

The tests draw with Mesa's software Vulkan driver, lavapipe, so their pictures come out the same
on any machine, and the window tests open their own Xvfb display. On Debian and Ubuntu:

```
sudo apt install mesa-vulkan-drivers xvfb
```

On Windows and macOS, the crates whose tests need no graphics card can be tested alone:

```
cargo test --locked -p gs-par -p gs-core -p gs-io -p gs-protocol
```

[cargo-nextest](https://nexte.st) runs the whole suite faster, every test as its own process:
`cargo nextest run --workspace --locked --test '*'`. Tests in a module named `tier2` are slow
checks; `-E 'not test(/(^|::)tier2::/)'` leaves them out.

## Quad Remesh's helper program

Mesh > Quad Remesh runs a separate program built from pinned open-source code (listed with its
exact versions in `third_party/quadwild/sources.lock`). Every release download carries it; a
build from source needs it built once. Without it, Quad Remesh says the program is
missing and where it looks, and the rest of the app works as usual.

You need git, CMake and a C++ compiler; on Linux also Ninja and bubblewrap (`bwrap`), which runs
the build and the program with no network; on Windows, Git Bash and Visual Studio's clang-cl
(macOS and Windows have no such sandbox, so there they run without one). Then, from the
repository (the folder is `~/Library/Caches/GigaSculpt/quadwild/$pin` on macOS and
`$LOCALAPPDATA/GigaSculpt/cache/quadwild/$pin` on Windows):

```
pin=$(awk '$1 == "pin" { print $2 }' third_party/quadwild/sources.lock)
dir=~/.cache/gigasculpt/quadwild/$pin
third_party/quadwild/fetch.sh "$dir"
third_party/quadwild/build.sh "$dir"
```

`fetch.sh` downloads the pinned sources and checks every file against the lock. It takes them
as one archive the project keeps beside its releases; if that cannot be had, it fetches each
piece from its own project instead, and the same check decides. `build.sh`
builds them (on Linux with the network off) and writes the program to `$dir/program`, where
GigaSculpt looks for it (with `$XDG_CACHE_HOME` set, under `$XDG_CACHE_HOME/gigasculpt` instead
of `~/.cache/gigasculpt`). GigaSculpt also finds it in a `quadwild` folder beside the
`gigasculpt` program (in the macOS app, in `Contents/Resources`), or in the folder `GS_QUADWILD`
names. The helper's own licenses are copied beside it.

The Quad Remesh tests need the helper too; without it they fail and say how to build it.

## Release downloads

`.github/workflows/release.yml` builds every download, then unpacks each on its own system and
starts it, as a person would. A tag such as `v0.1.0` publishes them; a push to a branch named
`release-trial/` and anything after it (for example `release-trial/1`) runs all the same but
publishes nothing, and keeps the downloads, named `vVERSION-trial.RUN`, with the run.

The version is one number: `version` under `[workspace.package]` in `Cargo.toml`, which both
programs print with `--version`. The Blender add-on repeats it in `blender_manifest.toml` and
`bl_info`, and a release tag must name it; `python3 addons/blender/tests/version_check.py`
checks the add-on (with `--tag v0.1.0`, a tag too).

The Linux download is built on Ubuntu 22.04 with GCC 12 (the helper is C++20), so it runs on
glibc 2.35 and later. A build from source on a newer system runs only there and later. The
Windows download carries Microsoft's C and C++ runtime DLLs beside its programs, copied from
Visual Studio's `VC\Redist` folder.
