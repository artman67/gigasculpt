//! Arguments from every source, flags, and exit codes.

use std::collections::BTreeSet;

use serde_json::json;

use crate::support::Env;

pub fn arguments_from_every_source() {
    let env = Env::new();
    env.spawn("args");
    env.ok(&[
        "call",
        "primitive.sphere",
        "--name",
        "ball",
        "--subdivisions",
        "4",
        "--center",
        "1,2,3",
        "radius=0.5",
    ]);
    let bounds = env
        .ok(&["query", "mesh.bounds", "part=ball", "--json"])
        .json();
    assert_eq!(
        bounds["ok"]["result"]["center"],
        json!([1.0, 2.0, 3.0]),
        "a comma list is a vector"
    );
    let size = &bounds["ok"]["result"]["size"][0];
    assert!(
        (size.as_f64().unwrap_or_default() - 1.0).abs() < 1e-6,
        "radius=0.5 is a number: {size}"
    );
    env.ok(&["call", "primitive.cube", r#"{"name": "box", "size": 3}"#]);
    env.ok(&["call", "primitive.cube", r#"{"name": "a=b"}"#]);
    let file = env.file("cube.json", r#"{"name": "filed", "subdivisions": 2}"#);
    env.ok(&["call", "primitive.cube", &format!("@{file}")]);
    let mesh = env.file(
        "tri.json",
        r#"{"positions": [[0,0,0],[1,0,0],[0,1,0]], "faces": [[0,1,2,4294967295]]}"#,
    );
    env.ok(&["call", "part.create", "name=tri", &format!("mesh=@{mesh}")]);
    env.ok(&["call", "primitive.cube", "name=123"]);
    let stats = env.ok(&["query", "mesh.stats", "--json"]).json();
    let names: BTreeSet<&str> = stats["ok"]["result"]["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["part"].as_str())
        .collect();
    assert_eq!(
        names,
        BTreeSet::from(["123", "a=b", "ball", "box", "filed", "tri"]),
        "every source of arguments"
    );
}

pub fn flags_and_bad_words() {
    let env = Env::new();
    env.spawn("flags");
    env.ok(&["call", "primitive.cube", "name=box"]);
    env.ok(&["call", "history.checkpoint", "name=c"]);
    env.ok(&["call", "history.checkpoint", "name=c", "--replace"]);
    assert_eq!(
        env.gs(&["call", "history.checkpoint", "name=c", "--replace=false"])
            .code,
        3,
        "flag set to false"
    );
    let moved = env
        .ok(&[
            "call",
            "mesh.translate",
            "part=box",
            "direction=0,0,1",
            "distance=10%",
            "--json",
        ])
        .json();
    assert_eq!(
        moved["ok"]["result"]["part"],
        json!("box"),
        "--json may follow the arguments"
    );
    assert_eq!(
        env.gs(&["call", "mesh.stats", "notapair"]).code,
        2,
        "a word that is not an argument"
    );
    assert_eq!(
        env.gs(&["call", "mesh.stats", "@/nonexistent.json"]).code,
        2,
        "a missing file"
    );
    assert_eq!(
        env.gs(&["call", "mesh.stats", "[1]"]).code,
        2,
        "JSON that is not an object"
    );
    assert_eq!(
        env.gs(&["call", "mesh.stats", "part=@/nonexistent.json"])
            .code,
        2,
        "a missing value file"
    );
}

pub fn exit_codes_say_what_went_wrong() {
    let env = Env::new();
    env.spawn("codes");
    let cases: [(&[&str], i32); 11] = [
        (&["call", "primitive.sphere", "subdivisions=0"], 2),
        (&["call", "primitve.sphere"], 2),
        (
            &[
                "call",
                "mesh.translate",
                "part=none",
                "direction=1,0,0",
                "distance=1",
            ],
            2,
        ),
        (&["call", "primitive.cube"], 0),
        (&["call", "primitive.cube", "name=cube"], 3),
        (&["call", "history.undo", "expect=999"], 3),
        (&["call", "debug.panic"], 1),
        (&["--role", "observer", "call", "primitive.sphere"], 3),
        (&["--role", "boss", "call", "primitive.sphere"], 2),
        (&["--role", "human", "call", "primitive.sphere"], 2),
        (&["call", "session.control", "paused=true"], 3),
    ];
    for (args, code) in cases {
        let out = env.gs(args);
        assert_eq!(out.code, code, "gs {args:?}:\n{}{}", out.stdout, out.stderr);
    }
    let paused = env.gs(&["call", "session.control", "paused=true"]);
    assert!(
        !paused.stderr.contains("--role") && paused.stderr.contains("human"),
        "says it is the human's control, not how to become one: {}",
        paused.stderr
    );
    env.ok(&["call", "primitive.sphere"]);
}

pub fn query_refuses_edits() {
    let env = Env::new();
    env.spawn("ro");
    let out = env.gs(&["query", "primitive.sphere"]);
    assert_eq!(out.code, 2, "{}", out.stderr);
    assert!(
        out.stderr.contains("gs call"),
        "says what to use: {}",
        out.stderr
    );
    let stats = env.ok(&["query", "mesh.stats", "--json"]).json();
    assert_eq!(
        stats["ok"]["result"]["parts"],
        json!([]),
        "nothing was made"
    );
}
