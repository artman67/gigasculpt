//! Bug bundles (decision 0031): every bundle kept in `tests/repros/` replays without its bug, a
//! panic writes a bundle that replays and minimizes, a bundle starts from the newest autosave
//! of its log run, and it carries the files its lines read.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

use crate::support::Env;

/// The bundles of fixed bugs, each replayed in a fresh engine: none of their bugs may come
/// back. Each is copied out first, since a replay works beside its bundle.
pub fn every_bundle_passes() {
    let kept = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/repros");
    let mut bundles: Vec<PathBuf> = fs::read_dir(&kept)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", kept.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "gsrepro"))
        .collect();
    bundles.sort();
    assert!(!bundles.is_empty(), "no bundles in {}", kept.display());
    let env = Env::new();
    for bundle in bundles {
        let copy = env.dir.join(bundle.file_name().unwrap_or_default());
        copy_folder(&bundle, &copy);
        let checked = env.ok(&["repro", "check", &copy.display().to_string()]);
        assert!(
            checked.lines().contains(&"bug: does not happen"),
            "the bug of {} is back:\n{}",
            bundle.display(),
            checked.stdout
        );
    }
}

/// A command that panics writes a bundle beside the session's log; it replays, minimizes to
/// the lines the panic needs, and only a fixed bug's bundle becomes a test.
pub fn panic_bundle_minimizes() {
    let env = Env::new();
    env.spawn("crashy");
    env.ok(&["call", "primitive.sphere", "subdivisions=3"]);
    env.ok(&["call", "primitive.cube"]);
    env.ok(&["call", "mesh.jitter", "amount=1%", "part=cube"]);
    let crashed = env.gs(&["call", "debug.panic", "part=sphere"]);
    assert_eq!(crashed.code, 1, "{}", crashed.stderr);
    let bundle = crashed
        .stderr
        .split("a bug bundle was written to ")
        .nth(1)
        .and_then(|rest| rest.split(';').next())
        .unwrap_or_else(|| panic!("the hint names the bundle: {}", crashed.stderr));
    assert!(
        bundle.starts_with(env.log_of("crashy").trim_end_matches(".jsonl")),
        "beside the log: {bundle}"
    );
    let checked = env.ok(&["repro", "check", bundle]);
    assert!(
        checked.lines().contains(&"bug: happens") && checked.lines().contains(&"line: 4"),
        "{}",
        checked.stdout
    );
    let small = env.dir.join("small.gsrepro").display().to_string();
    let shrunk = env.ok(&["repro", "minimize", bundle, "--out", &small]);
    assert!(
        shrunk.lines().contains(&"lines: 4 -> 2"),
        "{}",
        shrunk.stdout
    );
    let commands: Vec<String> = lines_of(Path::new(&small))
        .iter()
        .map(|line| line["command"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(
        commands,
        ["primitive.sphere", "debug.panic"],
        "the panic needs only its part"
    );
    let tests = env.dir.join("repros").display().to_string();
    let refused = env.gs(&["repro", "to-test", &small, "--into", &tests]);
    assert_eq!(refused.code, 3, "the bug still happens: {}", refused.stderr);
    // A line that fails some other way hides the bug rather than proving it gone.
    let info = Path::new(&small).join("bundle.json");
    let header = fs::read_to_string(&info).unwrap_or_default();
    let mut other: Value =
        serde_json::from_str(&header).unwrap_or_else(|e| panic!("bundle.json: {e}"));
    other["predicate"]["contains"] = json!("a message no panic has");
    fs::write(&info, other.to_string()).unwrap_or_else(|e| panic!("{e}"));
    let unknown = env.ok(&["repro", "check", &small]);
    assert!(
        unknown.lines().contains(&"bug: unknown"),
        "{}",
        unknown.stdout
    );
    let refused = env.gs(&["repro", "to-test", &small, "--into", &tests]);
    assert_eq!(refused.code, 3, "nothing to test: {}", refused.stderr);
    // Without the panicking line, the bundle stands for a fixed bug.
    fs::write(&info, header).unwrap_or_else(|e| panic!("{e}"));
    let commands = Path::new(&small).join("commands.jsonl");
    let first = fs::read_to_string(&commands).unwrap_or_default();
    let first = first.lines().next().unwrap_or_default().to_string();
    fs::write(&commands, first + "\n").unwrap_or_else(|e| panic!("{e}"));
    let kept = env.ok(&["repro", "to-test", &small, "--into", &tests]);
    let copy = Path::new(&tests).join("small.gsrepro");
    assert!(
        kept.stdout.contains(&copy.display().to_string()) && copy.join("bundle.json").exists(),
        "{}",
        kept.stdout
    );
}

/// With autosave on, a bundle starts from the newest autosave of the log run and holds only
/// the lines after it, and replaying it gives the scene the whole run gave.
pub fn bundles_start_from_the_newest_autosave() {
    let env = Env::new();
    let me = std::env::current_exe().unwrap_or_else(|e| panic!("no test program path: {e}"));
    let mut engine = Command::new(me)
        .args(["serve", "--headless", "--session", "saver", "--autosave"])
        .args(["--stop-key", "saver-key"])
        .env("XDG_RUNTIME_DIR", &env.dir)
        .env("XDG_STATE_HOME", env.dir.join("state"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("cannot start the engine: {e}"));
    env.keys
        .borrow_mut()
        .insert("saver".into(), "saver-key".into());
    // A busy machine can take well over 10 s to start an engine.
    for _ in 0..1800 {
        if env.sessions().iter().any(|s| s == "saver") {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    env.ok(&["call", "primitive.sphere", "subdivisions=3"]);
    env.ok(&["call", "mesh.jitter", "amount=1%"]);
    env.ok(&["call", "file.quick_save"]);
    env.ok(&[
        "call",
        "mesh.translate",
        "direction=[1,0,0]",
        "distance=0.5",
    ]);
    env.ok(&["call", "mesh.jitter", "amount=2%"]);
    let whole_run = env.hash("saver");
    let bundle = env.dir.join("after.gsrepro");
    let predicate = json!({ "kind": "hash_mismatch", "line": 2, "expected": whole_run });
    let saved = env.ok(&[
        "call",
        "repro.save",
        &format!("path={}", bundle.display()),
        &format!("predicate={predicate}"),
    ]);
    assert!(
        saved.lines().contains(&"result.start: file") && saved.lines().contains(&"result.lines: 2"),
        "{}",
        saved.stdout
    );
    let commands: Vec<Value> = lines_of(&bundle)
        .iter()
        .map(|l| l["command"].clone())
        .collect();
    assert_eq!(
        commands,
        [json!("mesh.translate"), json!("mesh.jitter")],
        "only the lines after the autosave"
    );
    let checked = env.ok(&["repro", "check", &bundle.display().to_string()]);
    assert!(
        checked.lines().contains(&"bug: does not happen"),
        "the replay from the autosave gives the whole run's scene: {}",
        checked.stdout
    );
    env.stop("saver");
    engine.wait().ok();
}

/// A bundle holds a copy of each file its lines read, the line pointing at it, so it replays
/// after the file is gone (review 0008, m4).
pub fn bundles_carry_the_files_their_lines_read() {
    let env = Env::new();
    env.spawn("reader");
    let corner = |k: usize| format!("v {} {} {}\n", k & 1, k >> 1 & 1, k >> 2 & 1);
    let faces = [
        "1 3 4 2", "5 6 8 7", "1 2 6 5", "3 7 8 4", "1 5 7 3", "2 4 8 6",
    ];
    let obj: String = (0..8)
        .map(corner)
        .chain(faces.iter().map(|f| format!("f {f}\n")))
        .collect();
    let read = env.dir.join("box.obj");
    fs::write(&read, obj).unwrap_or_else(|e| panic!("cannot write {}: {e}", read.display()));
    env.ok(&["call", "file.import", &format!("path={}", read.display())]);
    let bundle = env.dir.join("box.gsrepro");
    let expected = env.hash("reader");
    let predicate = json!({ "kind": "hash_mismatch", "line": 1, "expected": expected });
    env.ok(&[
        "call",
        "repro.save",
        &format!("path={}", bundle.display()),
        &format!("predicate={predicate}"),
    ]);
    fs::remove_file(&read).unwrap_or_else(|e| panic!("{e}"));
    let path = lines_of(&bundle)[0]["args"]["path"].clone();
    assert_eq!(path, json!("files/1/box.obj"), "the line reads the copy");
    let checked = env.ok(&["repro", "check", &bundle.display().to_string()]);
    assert!(
        checked.lines().contains(&"bug: does not happen"),
        "the replay imports the bundle's copy: {}",
        checked.stdout
    );
}

/// A bundle's command lines as JSON.
fn lines_of(bundle: &Path) -> Vec<Value> {
    let text = fs::read_to_string(bundle.join("commands.jsonl")).unwrap_or_default();
    (text.lines())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")))
        .collect()
}

fn copy_folder(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap_or_else(|e| panic!("cannot make {}: {e}", to.display()));
    for entry in fs::read_dir(from).into_iter().flatten().flatten() {
        let (path, copy) = (entry.path(), to.join(entry.file_name()));
        if path.is_dir() {
            copy_folder(&path, &copy);
            continue;
        }
        fs::copy(&path, copy).unwrap_or_else(|e| panic!("cannot copy {}: {e}", path.display()));
    }
}
