//! The source's tree digest (decision 0038) is the same whichever way the system's SHA-256 tool
//! marks its lines: "HASH  NAME" from Linux's `sha256sum` and macOS's `shasum`, "HASH *NAME"
//! from Windows' Git Bash, whose `sha256sum` reads in binary mode unless told otherwise. A
//! stand-in `sha256sum` asks the real tool for binary mode, which marks every line so here too.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::stand_in::put;

/// Windows' `sha256sum` as it prints: the real tool (found on `REAL_PATH`) in binary mode.
const BINARY_MODE: &str = "#!/bin/sh\nPATH=$REAL_PATH\n\
    if command -v sha256sum >/dev/null; then exec sha256sum -b \"$@\"; fi\n\
    exec shasum -a 256 -b \"$@\"\n";

/// `digest.sh` over `tree`, with `first` ahead of the usual search path when given.
fn digest(tree: &Path, first: Option<&Path>) -> String {
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../third_party/quadwild/digest.sh");
    let path = std::env::var_os("PATH").unwrap_or_default();
    let dirs = first.map(Path::to_path_buf).into_iter();
    let search = std::env::join_paths(dirs.chain(std::env::split_paths(&path)))
        .unwrap_or_else(|e| panic!("a search path: {e}"));
    let out = Command::new("bash")
        .arg(script)
        .arg(tree)
        .env("PATH", search)
        .env("REAL_PATH", &path)
        .output()
        .unwrap_or_else(|e| panic!("bash runs digest.sh: {e}"));
    assert!(out.status.success(), "digest.sh: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

#[test]
fn the_tree_digest_does_not_depend_on_the_tools_binary_mode_mark() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("quad-digest-{}", std::process::id()));
    let tree = dir.join("tree");
    put(&tree.join("a/one.txt"), "one\n");
    put(&tree.join("two words.txt"), "two\r\n");
    if !tree.join("link").exists() {
        std::os::unix::fs::symlink("a/one.txt", tree.join("link")).expect("a symbolic link");
    }
    let binary = dir.join("binary-mode");
    put(&binary.join("sha256sum"), BINARY_MODE);

    let plain = digest(&tree, None);
    assert_eq!(plain.len(), 64, "a SHA-256: {plain:?}");
    assert_eq!(
        digest(&tree, Some(&binary)),
        plain,
        "the digest with a tool that marks binary mode (\"HASH *NAME\")"
    );
}
