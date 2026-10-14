//! The pending queue: new pictures never overwrite a reference; approve, reject, revert, keep,
//! review and the digest move them as documented.

use crate::common::{goldens, lavapipe, scene_file, write_scene};
use gs_render::Image;
use gs_testkit::golden::{Goldens, Rule, Status, names, read, write};

fn cube(subdivisions: u32) -> String {
    scene_file(
        "\"zebra\", \"clay\"",
        "primitive.cube",
        &format!("{{ subdivisions = {subdivisions} }}"),
    )
}

fn statuses(g: &Goldens) -> Vec<(String, Status)> {
    let outcomes = g.check(lavapipe()).expect("scenes render");
    outcomes.into_iter().map(|o| (o.name, o.status)).collect()
}

fn image(dir: &std::path::Path, name: &str) -> Image {
    read(dir, name)
        .expect("reads")
        .unwrap_or_else(|| panic!("{name} in {}", dir.display()))
        .image
}

fn listed(dir: &std::path::Path) -> Vec<String> {
    names(dir).expect("lists")
}

#[test]
fn new_pictures_queue_and_never_overwrite_references() {
    let g = goldens("flow", &[("box", &cube(2))]);
    let (zebra, clay) = ("box.zebra".to_string(), "box.clay".to_string());
    let first = statuses(&g);
    assert!(
        first.iter().all(|(_, s)| *s == Status::New),
        "no references yet: {first:?}"
    );
    assert_eq!(
        listed(&g.pending()),
        [clay.clone(), zebra.clone()],
        "both queued"
    );
    assert!(
        listed(&g.reference()).is_empty(),
        "nothing written as a reference"
    );
    assert!(
        g.failures() == g.work.join("failures")
            && g.failures().join(&zebra).join("actual.png").exists(),
        "the new picture is in WORK/failures/ too"
    );
    let manifest = read(&g.pending(), &zebra)
        .expect("reads")
        .expect("queued")
        .manifest;
    assert_eq!(
        (
            manifest.scene.as_str(),
            manifest.look.as_str(),
            manifest.size
        ),
        ("box", "zebra", 512),
        "the manifest names scene, look and size"
    );
    assert_eq!(
        manifest.driver,
        lavapipe().gpu().driver(),
        "and the renderer"
    );
    g.approve(&[zebra.clone(), clay.clone()]).expect("approves");
    assert!(listed(&g.pending()).is_empty(), "the queue is empty");
    assert!(
        listed(&g.previous()).is_empty(),
        "new references replaced nothing"
    );
    let original = image(&g.reference(), &zebra);
    assert!(
        statuses(&g)
            .iter()
            .all(|(_, s)| matches!(s, Status::Matches(v) if v.rule == Rule::Bytes)),
        "the same renderer matches byte for byte"
    );
    write_scene(&g, "box", &cube(3));
    let changed = statuses(&g);
    assert!(
        changed
            .iter()
            .all(|(_, s)| matches!(s, Status::Changed(v) if !v.pass)),
        "a different cube changes both pictures: {changed:?}"
    );
    assert_eq!(
        image(&g.reference(), &zebra),
        original,
        "the reference is untouched"
    );
    for file in ["expected", "actual", "diff"] {
        let path = g.failures().join(&zebra).join(format!("{file}.png"));
        assert!(path.exists(), "{} is written", path.display());
    }
}

/// Approved references for the box scene, then the box changed and checked: both pictures
/// pending. Returns the folders and the original zebra reference.
fn approved_then_changed(name: &str) -> (Goldens, Image) {
    let g = goldens(name, &[("box", &cube(2))]);
    statuses(&g);
    g.approve(&["box.zebra".into(), "box.clay".into()])
        .expect("approves");
    let original = image(&g.reference(), "box.zebra");
    write_scene(&g, "box", &cube(3));
    statuses(&g);
    (g, original)
}

#[test]
fn approvals_keep_what_they_replace_until_kept_or_reverted() {
    let (g, original) = approved_then_changed("approvals");
    let (zebra, clay) = ("box.zebra".to_string(), "box.clay".to_string());
    g.approve(std::slice::from_ref(&zebra)).expect("approves");
    assert_eq!(
        image(&g.previous(), &zebra),
        original,
        "the replaced reference is kept"
    );
    write_scene(&g, "box", &cube(4));
    statuses(&g);
    g.approve(std::slice::from_ref(&zebra))
        .expect("approves again");
    assert_eq!(
        image(&g.previous(), &zebra),
        original,
        "before the maintainer looks, the oldest replaced reference stays"
    );
    let digest = g.work.join("digest.png");
    assert_eq!(
        g.digest(&digest).expect("digest"),
        1,
        "one changed reference"
    );
    let sheet = Image::from_png(&std::fs::read(&digest).expect("written")).expect("png");
    assert_eq!(
        (sheet.width, sheet.height),
        (3 * 512, 512),
        "old, new and difference"
    );
    g.revert(std::slice::from_ref(&zebra)).expect("reverts");
    assert_eq!(
        image(&g.reference(), &zebra),
        original,
        "the old reference is back"
    );
    assert!(
        listed(&g.previous()).is_empty(),
        "and no longer awaits a look"
    );
    g.approve(std::slice::from_ref(&clay))
        .expect("approves the clay picture");
    let approved = image(&g.reference(), &clay);
    g.keep(std::slice::from_ref(&clay)).expect("keeps");
    assert!(
        listed(&g.previous()).is_empty(),
        "kept: the old one is forgotten"
    );
    assert_eq!(
        image(&g.reference(), &clay),
        approved,
        "the approval stands"
    );
    assert_eq!(
        g.digest(&digest).expect("digest"),
        0,
        "nothing awaits a look"
    );
    statuses(&g);
    assert_eq!(
        listed(&g.pending()),
        std::slice::from_ref(&zebra),
        "the reverted picture differs again"
    );
    g.reject(std::slice::from_ref(&zebra)).expect("rejects");
    assert!(
        listed(&g.pending()).is_empty(),
        "rejected pictures are dropped"
    );
    assert_eq!(
        image(&g.reference(), &zebra),
        original,
        "rejecting leaves the reference"
    );
}

#[test]
fn moves_need_a_picture_to_move() {
    let g = goldens("missing", &[]);
    let name = ["nothing.zebra".to_string()];
    for (what, result) in [
        ("approve", g.approve(&name)),
        ("reject", g.reject(&name)),
        ("revert", g.revert(&name)),
        ("keep", g.keep(&name)),
    ] {
        assert!(
            result.is_err_and(|e| e.contains("nothing.zebra")),
            "{what} of a missing picture says which"
        );
    }
}

#[test]
fn review_reports_the_queue_and_mesh_checks() {
    let open = "{ name = \"lid\", mesh = { positions = [[0, 0, 0], [1, 0, 0], [0, 1, 0]], faces = [[0, 1, 2, 4294967295]] } }";
    let g = goldens(
        "review",
        &[
            ("box", &cube(2)),
            ("lid", &scene_file("\"clay\"", "part.create", open)),
        ],
    );
    statuses(&g);
    let lid = read(&g.pending(), "lid.clay")
        .expect("reads")
        .expect("queued");
    assert!(
        lid.manifest.defects.iter().any(|d| d.contains("lid")),
        "a scene that should be closed but is not records mesh check failures: {:?}",
        lid.manifest.defects
    );
    g.approve(&["box.zebra".into()]).expect("approves");
    write_scene(&g, "box", &cube(3));
    statuses(&g);
    let report = g.review().expect("reviews");
    for expected in [
        "pending (3):",
        "box.zebra: fails the Bytes rule",
        "box.clay: new, no reference",
        "mesh check: lid:",
        "review.png",
        "awaiting the maintainer's look (0)",
    ] {
        assert!(
            report.contains(expected),
            "review mentions {expected:?}:\n{report}"
        );
    }
    let sheet =
        Image::from_png(&std::fs::read(g.work.join("review.png")).expect("written")).expect("png");
    assert_eq!(
        (sheet.width, sheet.height),
        (3 * 512, 3 * 512),
        "a row per pending picture"
    );
    g.approve(&["box.zebra".into()]).expect("approves");
    let report = g.review().expect("reviews");
    assert!(
        report.contains("awaiting the maintainer's look (1): box.zebra"),
        "approvals await a look:\n{report}"
    );
}

#[test]
fn scene_files_are_checked() {
    let bad = [
        ("look", scene_file("\"chrome\"", "primitive.cube", "{}")),
        ("matcap", scene_file("\"matcap:\"", "primitive.cube", "{}")),
        (
            "view",
            "about = \"x\"\nview = \"under\"\nlooks = [\"clay\"]\n[[step]]\ncommand = \"primitive.cube\"\n"
                .into(),
        ),
        ("steps", "about = \"x\"\nlooks = [\"clay\"]\nstep = []\n".into()),
        ("field", format!("colour = 1\n{}", scene_file("\"clay\"", "primitive.cube", "{}"))),
    ];
    for (what, text) in bad {
        assert!(
            gs_testkit::golden::GoldenScene::parse(what, &text).is_err_and(|e| e.starts_with(what)),
            "a bad {what} is refused, naming the scene"
        );
    }
    let failing = scene_file("\"clay\"", "primitive.cube", "{ subdivisions = 0 }");
    let scene = gs_testkit::golden::GoldenScene::parse("zero", &failing).expect("parses");
    assert!(
        scene
            .run()
            .is_err_and(|e| e.contains("step 1 (primitive.cube) failed")),
        "a failing step is named"
    );
}

/// A reference rendered on another renderer is held to the cross-renderer rule of its look:
/// two pixels far off still pass a zebra picture but fail a clay one (FLIP past 0.15).
#[test]
fn references_from_another_renderer_get_their_looks_rule() {
    let g = goldens("other-renderer", &[("box", &cube(2))]);
    statuses(&g);
    for name in ["box.zebra", "box.clay"] {
        let mut picture = read(&g.pending(), name).expect("reads").expect("queued");
        for x in [250u32, 252] {
            let i = ((256 * picture.image.width + x) * 4) as usize;
            for c in &mut picture.image.rgba[i..i + 3] {
                *c = c.saturating_add(60);
            }
        }
        picture.manifest.adapter = "another adapter".into();
        write(&g.reference(), name, &picture).expect("writes");
    }
    let got = statuses(&g);
    let status = |name: &str| got.iter().find(|s| s.0 == name).map(|s| s.1.clone());
    assert!(
        matches!(status("box.zebra"), Some(Status::Matches(v)) if v.rule == Rule::Zebra && v.differing == 2),
        "the zebra picture passes the zebra rule: {got:?}"
    );
    assert!(
        matches!(status("box.clay"), Some(Status::Changed(v)) if v.rule == Rule::Look),
        "the clay picture fails the look rule: {got:?}"
    );
}

/// A damaged picture or manifest is an error that names the file.
#[test]
fn damaged_files_are_named() {
    let g = goldens("damaged", &[("box", &cube(2))]);
    statuses(&g);
    let dir = g.pending();
    std::fs::write(dir.join("box.zebra.png"), b"not a png").expect("damaged");
    assert!(
        read(&dir, "box.zebra").is_err_and(|e| e.contains("box.zebra.png")),
        "a bad picture is named"
    );
    std::fs::write(dir.join("box.clay.toml"), "scene = 1").expect("damaged");
    assert!(
        read(&dir, "box.clay").is_err_and(|e| e.contains("box.clay.toml")),
        "a bad manifest is named"
    );
    std::fs::write(dir.join("notes.txt"), "not a picture").expect("written");
    assert_eq!(
        names(&dir).expect("lists"),
        ["box.clay", "box.zebra"],
        "only pictures are listed"
    );
}
