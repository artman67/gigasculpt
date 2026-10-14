//! Where the helper's source comes from (decision 0038, amended 2026-10-14 (Release-prep)):
//! `fetch.sh` unpacks the project's own archive of the assembled tree when it can, else takes
//! each of the lock's lines from its own home, and uses a source only when its tree is the one
//! the lock names. Each test runs the real scripts beside a lock of its own, on a small tree
//! whose commit, tarball and archives sit in the test's folder behind `file://` URLs, so nothing
//! touches the network; an unreachable source is a `file://` URL to nothing.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use super::stand_in::put;

/// git with no settings of the machine's or the user's, and a name to commit with.
const GIT_ENV: [(&str, &str); 6] = [
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_AUTHOR_NAME", "test"),
    ("GIT_AUTHOR_EMAIL", "test@example.com"),
    ("GIT_COMMITTER_NAME", "test"),
    ("GIT_COMMITTER_EMAIL", "test@example.com"),
];

/// `third_party/quadwild/NAME`.
fn script(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../third_party/quadwild")
        .join(name)
}

/// Runs `program` in `dir`; it must succeed. Its output, trimmed.
fn run(dir: &Path, program: &str, args: &[&str]) -> String {
    let out = Command::new(program)
        .args(args)
        .current_dir(dir)
        .envs(GIT_ENV)
        .output()
        .unwrap_or_else(|e| panic!("cannot run {program}: {e}"));
    assert!(out.status.success(), "{program} {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A test's folder: a git repository and a tarball, the lines' homes (`upstream/`); the source
/// they assemble, made by hand (`made/src`), and its archive; another tree's archive; the
/// scripts beside the test's lock (`scripts/`); and what `fetch.sh` leaves (`out/`).
struct Fixture {
    dir: PathBuf,
    /// The commit of `upstream/repo` the git line names.
    commit: String,
    /// The SHA-256 of `upstream/extra-1.0.tar.gz`, the tar line's tarball.
    tarball: String,
    /// The SHA-256 of `archive.tar.gz`, the archive of `made/src`.
    archive: String,
    /// The SHA-256 of `other.tar.gz`, the archive of another tree.
    other: String,
    /// `made/src`'s digest: the lock's tree line.
    tree: String,
}

impl Fixture {
    fn new(name: &str) -> Fixture {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("quad-fetch-{name}-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        let repo = dir.join("upstream/repo");
        put(&repo.join("README"), "the top\n");
        put(&repo.join("libs/one.txt"), "one\r\n");
        run(&repo, "git", &["init", "-q"]);
        run(&repo, "git", &["add", "-A"]);
        run(&repo, "git", &["commit", "-q", "-m", "one"]);
        put(&dir.join("upstream/extra-1.0/c.txt"), "extra\n");
        let upstream = dir.join("upstream");
        run(&upstream, "tar", &["-czf", "extra-1.0.tar.gz", "extra-1.0"]);
        for (file, text) in [("README", "the top\n"), ("libs/one.txt", "one\r\n")] {
            put(&dir.join("made/src").join(file), text);
        }
        put(&dir.join("made/src/extra/c.txt"), "extra\n");
        put(&dir.join("other/src/README"), "another commit's top\n");
        run(
            &dir.join("made"),
            "tar",
            &["-czf", "../archive.tar.gz", "src"],
        );
        run(
            &dir.join("other"),
            "tar",
            &["-czf", "../other.tar.gz", "src"],
        );
        let sum = |file: &str| {
            let said = run(&dir, "bash", &["-c", SHA256, "-", file]);
            said.split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned()
        };
        Fixture {
            commit: run(&repo, "git", &["rev-parse", "HEAD"]),
            tarball: sum("upstream/extra-1.0.tar.gz"),
            archive: sum("archive.tar.gz"),
            other: sum("other.tar.gz"),
            tree: digest(&dir.join("made/src")),
            dir,
        }
    }

    /// A lock naming the made tree, whose archive line gives `archive` (a file in the folder)
    /// with the SHA-256 `sum`, and whose lines' homes are reachable or not.
    fn lock(&self, archive: &str, sum: &str, reachable: bool) -> String {
        let home = if reachable { "upstream" } else { "nowhere" };
        let url = |name: &str| format!("file://{}", self.dir.join(name).display());
        let (repo, tarball) = (
            url(&format!("{home}/repo")),
            url(&format!("{home}/extra-1.0.tar.gz")),
        );
        format!(
            "pin test-1\ngit . {} {repo}\ntar extra {} {tarball}\narchive {sum} {}\ntree {}\n",
            self.commit,
            self.tarball,
            url(archive),
            self.tree
        )
    }

    /// `fetch.sh` into `out`, run beside `lock`: whether it succeeded, and all it said.
    fn fetch(&self, lock: &str) -> (bool, String) {
        let scripts = self.dir.join("scripts");
        put(&scripts.join("sources.lock"), lock);
        for name in ["fetch.sh", "digest.sh"] {
            fs::copy(script(name), scripts.join(name))
                .unwrap_or_else(|e| panic!("cannot copy {name}: {e}"));
        }
        let out: Output = Command::new("bash")
            .arg(scripts.join("fetch.sh"))
            .arg(self.dir.join("out"))
            .envs(GIT_ENV)
            .output()
            .unwrap_or_else(|e| panic!("bash runs fetch.sh: {e}"));
        let said = String::from_utf8_lossy(&out.stdout).into_owned()
            + &String::from_utf8_lossy(&out.stderr);
        (out.status.success(), said)
    }

    /// The digest of the source `fetch.sh` left, if it left one.
    fn fetched(&self) -> Option<String> {
        let src = self.dir.join("out/src");
        src.exists().then(|| digest(&src))
    }
}

/// The SHA-256 of a file, by the tool each system has (as `fetch.sh` picks it).
const SHA256: &str = "if command -v sha256sum >/dev/null; then sha256sum \"$1\"; \
    else shasum -a 256 \"$1\"; fi";

/// `digest.sh` over `tree`.
fn digest(tree: &Path) -> String {
    let (script, path) = (script("digest.sh"), tree.to_string_lossy());
    run(tree, "bash", &[&*script.to_string_lossy(), &*path])
}

/// The university server down (run 38533123869): no line's home answers, and the archive
/// gives the tree alone, without trying them.
#[test]
fn the_archive_gives_the_tree_when_no_line_can_be_reached() {
    let fx = Fixture::new("archive");
    let (ok, said) = fx.fetch(&fx.lock("archive.tar.gz", &fx.archive, false));
    assert!(ok, "fetch.sh: {said}");
    assert_eq!(
        fx.fetched(),
        Some(fx.tree.clone()),
        "the lock's tree: {said}"
    );
    assert!(
        said.contains("from the archive") && !said.contains("fetching"),
        "the archive alone was used: {said}"
    );
}

#[test]
fn an_unreachable_archive_falls_back_to_the_lines_and_assembles_the_same_tree() {
    let fx = Fixture::new("fallback");
    let (ok, said) = fx.fetch(&fx.lock("nowhere.tar.gz", &fx.archive, true));
    assert!(ok, "fetch.sh: {said}");
    assert_eq!(
        fx.fetched(),
        Some(fx.tree.clone()),
        "the lock's tree: {said}"
    );
    assert!(
        said.contains("passed over the archive") && said.contains("from the lock's lines"),
        "the lines were used after the archive: {said}"
    );
}

#[test]
fn an_archive_whose_checksum_is_wrong_is_refused() {
    let fx = Fixture::new("checksum");
    let (ok, said) = fx.fetch(&fx.lock("archive.tar.gz", &"0".repeat(64), false));
    assert!(!ok, "refused: {said}");
    assert!(
        said.contains(&format!("has SHA-256 {}, the lock says", fx.archive)),
        "the archive's checksum is named: {said}"
    );
    assert_eq!(fx.fetched(), None, "no source was left: {said}");
}

/// The archive holds another commit's tree (its own checksum right), and the git line's home
/// lacks the commit the lock names: neither is used.
#[test]
fn a_source_holding_another_commit_is_refused() {
    let fx = Fixture::new("commit");
    let lock = fx.lock("other.tar.gz", &fx.other, true);
    let (ok, said) = fx.fetch(&lock.replace(&fx.commit, &"1".repeat(40)));
    assert!(!ok, "refused: {said}");
    assert!(
        said.contains(&format!(
            "the archive's tree digest is {}",
            digest(&fx.dir.join("other/src"))
        )) && said.contains(&"1".repeat(40)),
        "the archive's tree and the missing commit are named: {said}"
    );
    assert_eq!(fx.fetched(), None, "no source was left: {said}");
}
