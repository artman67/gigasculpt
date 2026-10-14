//! The `golden` tool, through its library entry point and its binary.

use crate::common::{goldens, scene_file};
use gs_testkit::golden::cli::{USAGE, run};
use gs_testkit::golden::{Goldens, names};

fn golden(g: &Goldens, words: &[&str]) -> Result<String, String> {
    let root = g.root.to_string_lossy().into_owned();
    let work = g.work.to_string_lossy().into_owned();
    let mut args = vec!["--root".to_string(), root, "--work".into(), work];
    args.extend(words.iter().map(|w| (*w).to_string()));
    run(&args)
}

#[test]
fn the_tool_runs_every_command() {
    let scene = scene_file("\"clay\"", "primitive.cube", "{ subdivisions = 2 }");
    let g = goldens("cli", &[("box", &scene)]);
    let first = golden(&g, &["check"]);
    assert!(
        first
            .as_ref()
            .is_err_and(|e| e.contains("box.clay is new") && e.contains("0 match")),
        "check reports new pictures as a failure: {first:?}"
    );
    let review = golden(&g, &["review"]).expect("reviews");
    assert!(
        review.contains("pending (1):"),
        "review lists the queue: {review}"
    );
    let request = g.work.join("request.toml");
    std::fs::write(
        &request,
        "item = \"x\"\nexpect = \"a cube\"\nscenes = [\"box\"]\n",
    )
    .expect("written");
    let decided = golden(&g, &["decide", &request.to_string_lossy()]).expect("decides");
    assert_eq!(
        decided, "Approve box.clay: the change x expects",
        "one line per picture"
    );
    assert_eq!(
        golden(&g, &["approve", "--all"]),
        Ok("approved 1: box.clay".into()),
        "approve --all takes the whole queue"
    );
    assert_eq!(
        golden(&g, &["check"]),
        Ok("1 pictures, 1 match their references".into()),
        "then everything matches"
    );
}

#[test]
fn the_tool_moves_references_and_makes_the_digest() {
    let scene = scene_file("\"clay\"", "primitive.cube", "{ subdivisions = 2 }");
    let g = goldens("cli-moves", &[("box", &scene)]);
    golden(&g, &["check"]).expect_err("new");
    golden(&g, &["approve", "--all"]).expect("approves");
    std::fs::write(
        g.scenes().join("box.toml"),
        scene_file("\"clay\"", "primitive.cube", "{ subdivisions = 3 }"),
    )
    .expect("scene changed");
    assert!(
        golden(&g, &["check"]).is_err(),
        "a changed cube fails the check"
    );
    assert_eq!(
        golden(&g, &["reject", "box.clay"]),
        Ok("rejected 1: box.clay".into()),
        "reject"
    );
    golden(&g, &["check"]).expect_err("queued again");
    assert_eq!(
        golden(&g, &["approve", "box.clay"]),
        Ok("approved 1: box.clay".into()),
        "approve"
    );
    let digest = golden(&g, &["digest"]).expect("digest");
    assert!(
        digest.starts_with("1 changed references: ") && g.work.join("digest.png").exists(),
        "the digest goes to WORK/digest.png by default: {digest}"
    );
    let elsewhere = g.work.join("sheet.png");
    golden(&g, &["digest", &elsewhere.to_string_lossy()]).expect("digest");
    assert!(elsewhere.exists(), "or where asked");
    assert_eq!(
        golden(&g, &["revert", "box.clay"]),
        Ok("reverted 1: box.clay".into()),
        "revert"
    );
    golden(&g, &["check"]).expect_err("queued again");
    golden(&g, &["approve", "--all"]).expect("approves");
    assert_eq!(
        golden(&g, &["keep", "--all"]),
        Ok("kept 1: box.clay".into()),
        "keep --all"
    );
    assert!(
        names(&g.previous()).expect("lists").is_empty(),
        "nothing awaits a look"
    );
    assert_eq!(
        golden(&g, &["digest"]),
        Ok("no changed references".into()),
        "an empty digest"
    );
}

#[test]
fn the_tool_explains_its_usage() {
    let g = goldens("cli-usage", &[]);
    assert_eq!(
        golden(&g, &["help"]),
        Ok(USAGE.into()),
        "help prints the usage"
    );
    for wrong in [
        &["approve"][..],
        &["revert", "--all"],
        &["digest", "a", "b"],
        &["decide"],
        &["frobnicate"],
        &[],
    ] {
        assert_eq!(
            golden(&g, wrong),
            Err(USAGE.into()),
            "{wrong:?} prints the usage"
        );
    }
    assert_eq!(
        run(&["--root".into()]),
        Err(USAGE.into()),
        "a flag needs its value"
    );
    assert!(
        golden(&g, &["decide", "/no/such/request.toml"]).is_err_and(|e| e.contains("request.toml")),
        "an unreadable request names the file"
    );
}

#[test]
fn the_binary_prints_and_exits_by_outcome() {
    let g = goldens("binary", &[]);
    let tool = |words: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_golden"))
            .args([
                "--root",
                &g.root.to_string_lossy(),
                "--work",
                &g.work.to_string_lossy(),
            ])
            .args(words)
            .output()
            .expect("the tool runs")
    };
    let ok = tool(&["review"]);
    assert!(ok.status.success(), "review succeeds");
    assert!(
        String::from_utf8_lossy(&ok.stdout).starts_with("pending (0):"),
        "and prints its report"
    );
    let bad = tool(&["frobnicate"]);
    assert!(!bad.status.success(), "an unknown command fails");
    assert!(
        String::from_utf8_lossy(&bad.stdout).contains("COMMAND"),
        "and prints the usage"
    );
}
