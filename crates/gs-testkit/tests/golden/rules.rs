//! The automated approval rules of ARCHITECTURE 10.3.

use crate::common::{goldens, lavapipe};
use gs_render::Image;
use gs_testkit::golden::{Decision, Manifest, Request, Ruling, Stored, decide, write};

fn manifest(scene: &str, defects: &[&str]) -> Manifest {
    let mut m = Manifest::new(lavapipe().gpu(), scene, "zebra", Vec::new());
    m.defects = defects.iter().map(|d| (*d).to_string()).collect();
    m
}

fn request(expect: &str, scenes: &[&str], artistic: bool) -> Request {
    Request {
        item: "brushes.clay".into(),
        expect: expect.into(),
        scenes: scenes.iter().map(|s| (*s).to_string()).collect(),
        artistic,
    }
}

fn rulings(request: &Request, pending: &[(&str, Manifest)]) -> Vec<(String, Ruling, String)> {
    let pending: Vec<(String, Manifest)> = (pending.iter())
        .map(|(n, m)| ((*n).to_string(), m.clone()))
        .collect();
    decide(request, &pending)
        .into_iter()
        .map(
            |Decision {
                 name,
                 ruling,
                 reason,
             }| (name, ruling, reason),
        )
        .collect()
}

/// Pending pictures as (name, manifest).
type Pending = Vec<(&'static str, Manifest)>;

fn clean(scene: &str) -> Manifest {
    manifest(scene, &[])
}

fn wanted() -> Request {
    request("the dent gets deeper", &["dent", "jitter"], false)
}

fn both() -> Pending {
    vec![
        ("dent.zebra", clean("dent")),
        ("jitter.zebra", clean("jitter")),
    ]
}

/// Situations where every picture of the change gets one ruling: what, the request, the
/// ruling and words its reason must contain.
fn whole_change_cases() -> Vec<(&'static str, Request, Ruling, &'static str)> {
    vec![
        (
            "the expected change",
            wanted(),
            Ruling::Approve,
            "the change brushes.clay expects",
        ),
        (
            "no expectation was stated",
            request(" ", &["dent", "jitter"], false),
            Ruling::Reject,
            "does not say what visual change to expect",
        ),
        (
            "an affected scene did not change",
            request("deeper", &["dent", "jitter", "sphere"], false),
            Ruling::Reject,
            "missing from sphere",
        ),
        (
            "an artistic change",
            request("a softer clay look", &["dent", "jitter"], true),
            Ruling::Escalate,
            "the maintainer decides",
        ),
    ]
}

#[test]
fn approval_follows_the_rules() {
    for (what, request, ruling, why) in whole_change_cases() {
        let got = rulings(&request, &both());
        assert!(
            got.iter().all(|r| r.1 == ruling && r.2.contains(why)),
            "{what}: every picture gets {ruling:?} because {why:?}: {got:?}"
        );
    }
}

/// A regression or a failed mesh check is ruled on the picture it is in.
#[test]
fn regressions_and_broken_meshes_are_rejected_alone() {
    let mut extra = both();
    extra.insert(0, ("cube.zebra", clean("cube")));
    let got = rulings(&wanted(), &extra);
    assert_eq!(
        got.iter().map(|r| r.1).collect::<Vec<_>>(),
        [Ruling::Reject, Ruling::Approve, Ruling::Approve],
        "only the unrelated picture is rejected: {got:?}"
    );
    assert!(got[0].2.contains("regression"), "and called a regression");
    let mut broken = both();
    broken[0].1 = manifest("dent", &["sphere: hole"]);
    let got = rulings(&wanted(), &broken);
    assert_eq!(
        (got[0].1, got[1].1),
        (Ruling::Reject, Ruling::Approve),
        "only the broken picture is rejected: {got:?}"
    );
    assert!(
        got[0].2.contains("mesh checks fail: sphere: hole"),
        "naming the defect"
    );
}

#[test]
fn requests_are_read_from_toml_and_pending_from_disk() {
    let text =
        "item = \"geometry.voxel-remesh\"\nexpect = \"rounder\"\nscenes = [\"voxel-cube\"]\n";
    let req = Request::parse(text).expect("parses");
    assert_eq!(
        req,
        Request {
            item: "geometry.voxel-remesh".into(),
            expect: "rounder".into(),
            scenes: vec!["voxel-cube".into()],
            artistic: false,
        },
        "fields as written, artistic off by default"
    );
    assert!(
        Request::parse("item = \"x\"\nexpcet = \"typo\"\n").is_err(),
        "an unknown field is refused"
    );
    let g = goldens("decide", &[]);
    let picture = Stored {
        image: Image::filled(2, 2, [1, 2, 3, 255]),
        manifest: manifest("voxel-cube", &[]),
    };
    write(&g.pending(), "voxel-cube.zebra", &picture).expect("writes");
    let decided = g.decide(&req).expect("decides");
    assert_eq!(decided.len(), 1, "one pending picture");
    assert_eq!(
        (decided[0].name.as_str(), decided[0].ruling),
        ("voxel-cube.zebra", Ruling::Approve),
        "read from the queue"
    );
}
