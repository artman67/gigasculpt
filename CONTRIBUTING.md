# Contributing to GigaSculpt

Thank you for helping. This page says how to build and test GigaSculpt, the rules the code
follows, and how to report a bug.

## Build and run

[docs/building.md](docs/building.md) lists what each system needs. In short, with Rust
installed through rustup, CMake and a C compiler:

```
cargo build --locked
target/debug/gigasculpt
```

The repository's `rust-toolchain.toml` picks the Rust version, with clippy and rustfmt.

## Checks

A pull request has to pass the same checks the CI runs:

```
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo deny check licenses bans sources
cargo test --workspace --locked
```

`cargo deny` is a separate tool (`cargo install --locked cargo-deny`). The tests run faster
under [cargo-nextest](https://nexte.st), which runs every test as its own process, many at
once:

```
cargo nextest run --workspace --locked --test '*'
```

On Linux the tests need Mesa's software Vulkan driver (lavapipe), so they draw the same pictures
on any machine, and Xvfb for the window tests. The Quad Remesh tests need its helper program;
[docs/building.md](docs/building.md) says how to build it. Tests in a module named `tier2` are
slow checks; the CI leaves them out with `-E 'not test(/(^|::)tier2::/)'`.

## Rules for the code

- **Every action is a command.** Anything the app can do is a command in the registry, with a
  name, typed arguments and examples, so the window, `gs`, scripts and agents all reach it the
  same way. No feature only the interface can reach.
- **Brushes are presets.** A brush is a TOML file in `assets/brushes/` that sets up one of a
  few sculpting kernels. A new kernel is a bigger change: open an issue first.
- **One home per idea.** Falloff, mirroring, masks, undo capture and the shared math each live
  in one place. Search before you write; never copy a block of code to change it a little.
- **Determinism.** The same commands give the same bits on every run and at any thread count:
  no `HashMap` or `HashSet` (use `BTreeMap`, `BTreeSet` or `IndexMap`), no clock reads outside
  the engine's clock module, no platform `sin`, `cos`, `exp` and friends (use the shared math
  module), and parallel work only through `gs-par`. `clippy.toml` enforces most of this.
- **Size.** A function has at most 80 lines and a source file at most 1,000. No `unsafe`.
  `unwrap` and `expect` are for tests only; elsewhere, return an error that says what went
  wrong.
- **Tests drive commands.** Tests live in each crate's `tests/` folder, never in `src/`, and
  run commands the way a user or a script would, then check what they observe. No mocks of our
  own code. Good tests here are properties every command keeps (finite points, undo and redo
  exact, the mask respected, symmetry kept), relations (rotate the mesh and the stroke together
  and the result rotates too), a fast path checked against a slow, simple one, and golden
  pictures. A test that no longer fits the design is rewritten or deleted, never hidden with
  `#[ignore]`.
- **Names.** Use GigaSculpt's own names for things in code, the interface and docs.
- **Licenses.** Every dependency must be compatible with the GPL, version 3 or later;
  `cargo deny` checks it. Code may be ported from projects under a compatible license (for
  example GPL-2.0-or-later or MIT) with credit; never from leaked or decompiled code.

## Sending a change

Fork the repository, make your change on a branch, and open a pull request against `main`.
Say what it fixes or adds and how you checked it. Keep a pull request to one change; small ones
are reviewed sooner. For a bigger feature, open an issue first so we can agree on the design.

## Reporting a bug

Open an issue with the bug template. Please include:

- the version (`gigasculpt --version`), your system and your graphics card;
- what you did, what you expected and what happened;
- the session's command log if you can: the window writes every command to
  `logs/` under `$XDG_STATE_HOME/gigasculpt` (usually `~/.local/state/gigasculpt/logs/`), and
  `gs run` replays it into a fresh session, which is often all we need to see the bug.

## License

GigaSculpt is under the GNU General Public License, version 3 or later. By sending a pull
request you agree that your contribution is released under the same license.
