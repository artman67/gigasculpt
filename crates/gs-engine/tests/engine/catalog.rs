//! Registry properties: rules every command obeys, checked on every command in the catalog, so
//! a new command is covered the moment it is registered.

use std::collections::{BTreeMap, BTreeSet};

use gs_protocol::{AttrValues, Attribute, Catalog, CommandInfo, Effect, Kind, PlainMesh};
use gs_testkit::{Scene, Session, code, failure, reference, success, text};

use crate::support::{agent, human};
use serde_json::{Value, json};

fn catalog() -> Catalog {
    agent().engine.catalog().clone()
}

/// A session for exercising `info`: as an agent, unless only the human may run it.
fn session_for(info: &CommandInfo) -> Session {
    if info.human_only { human() } else { agent() }
}

fn properties(info: &CommandInfo) -> Vec<(String, Value)> {
    let found = info
        .args_schema
        .get("properties")
        .and_then(Value::as_object);
    found
        .into_iter()
        .flatten()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

#[test]
fn names_are_unique_dotted_lowercase_and_sorted() {
    let names: Vec<String> = catalog().commands.into_iter().map(|c| c.name).collect();
    assert!(
        names.len() >= 20,
        "non-trivial: the registry found {} commands",
        names.len()
    );
    for pair in names.windows(2) {
        assert!(
            pair[0] < pair[1],
            "names are not sorted and unique: {pair:?}"
        );
    }
    let word = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c == '_');
    for name in &names {
        let (noun, verb) = name.split_once('.').unwrap_or((name, ""));
        assert!(
            word(noun) && word(verb),
            "{name} is not lowercase noun.verb"
        );
    }
}

#[test]
fn every_command_and_argument_is_documented() {
    for info in catalog().commands {
        assert!(!info.docs.trim().is_empty(), "{} has no docs", info.name);
        assert_eq!(
            info.summary,
            info.docs.lines().next().unwrap_or_default(),
            "{} summary",
            info.name
        );
        assert_eq!(
            info.args_schema["description"],
            json!(info.docs),
            "{} schema docs",
            info.name
        );
        for (arg, schema) in properties(&info) {
            let doc = schema
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default();
            assert!(
                !doc.trim().is_empty(),
                "{}: argument {arg} has no docs",
                info.name
            );
        }
    }
}

/// Every command has a title a person reads, given where it is registered: what an undo step
/// is called (decision 0016, section 16). A title reads as a menu item does, capitalised, with
/// no name's dots or underscores. Fails naming each command without one.
#[test]
fn every_command_has_a_title_a_person_reads() {
    let specs = gs_engine::registry();
    let untitled: Vec<&str> = (specs.iter())
        .filter(|spec| {
            let first = spec.title.chars().next();
            !first.is_some_and(char::is_uppercase) || spec.title.contains(['.', '_'])
        })
        .map(|spec| spec.name)
        .collect();
    assert!(specs.len() >= 100, "non-trivial: {} commands", specs.len());
    assert!(
        untitled.is_empty(),
        "commands without a title: {untitled:?}"
    );
}

/// Robustness: an unknown argument, or any argument of the wrong type, is refused with a
/// pointer to it, and the scene is untouched.
#[test]
fn bad_arguments_are_refused_with_their_path_and_change_nothing() {
    for info in catalog().commands {
        let mut s = session_for(&info);
        s.ok("primitive.sphere", json!({ "subdivisions": 2 }));
        let before = s.hash();
        let err = s.err(&info.name, json!({ "zz_unknown": 1 }));
        assert_eq!(
            code(err.code),
            "E_ARGS_INVALID",
            "{} took an unknown argument",
            info.name
        );
        let pointed = err.violations.iter().any(|v| v.path == "/zz_unknown");
        assert!(
            pointed,
            "{}: no violation points at the unknown argument",
            info.name
        );
        for (arg, _) in properties(&info) {
            let err = s.err(&info.name, json!({ arg.clone(): { "wrong": ["type"] } }));
            assert_eq!(
                code(err.code),
                "E_ARGS_INVALID",
                "{}: mistyped {arg}",
                info.name
            );
            let pointed = err
                .violations
                .iter()
                .any(|v| v.path.starts_with(&format!("/{arg}")));
            assert!(
                pointed,
                "{}: no violation points at /{arg}: {:?}",
                info.name, err.violations
            );
        }
        assert_eq!(
            s.hash(),
            before,
            "{} changed the scene with bad arguments",
            info.name
        );
    }
}

/// Every example runs as declared; every edit example is a real change whose effect matches an
/// independent diff of the parts, and undo and redo restore it bit for bit.
#[test]
fn examples_run_and_edits_undo_and_redo_exactly() {
    let mut edits = 0;
    for info in catalog().commands {
        assert!(
            !info.examples.is_empty(),
            "{} has no example to copy",
            info.name
        );
        for example in &info.examples {
            let what = format!("{} example '{}'", info.name, example.summary);
            let mut s = session_for(&info);
            for step in &example.setup {
                s.ok(&step.command, step.args.clone());
            }
            let (before, old_parts) = (s.hash(), s.scene());
            let response = s.call(&info.name, example.args.clone());
            if let Some(expected) = example.fails_with {
                assert_eq!(failure(response, &what).code, expected, "{what}");
                assert_eq!(s.hash(), before, "{what} failed but changed the scene");
                continue;
            }
            let done = success(response, &what);
            if info.kind != Kind::Edit {
                continue;
            }
            edits += 1;
            let after = s.hash();
            assert_ne!(after, before, "{what} changed nothing");
            let effect = done
                .effect
                .unwrap_or_else(|| panic!("{what} reported no effect"));
            let new_parts = s.scene();
            check_effect(&effect, &old_parts, &new_parts, &what);
            // Only a change `part.get` cannot show (a part's detail levels around its active
            // level, decision 0014) touches no vertex; the hash above saw it.
            let shown =
                serde_json::to_value(&new_parts).ok() != serde_json::to_value(&old_parts).ok();
            assert!(
                effect.verts_touched > 0 || !shown,
                "{what} shows a change to vertices"
            );
            s.ok("history.undo", json!({ "expect": done.history_id }));
            assert_eq!(s.hash(), before, "undo of {what}");
            s.ok("history.redo", json!({ "expect": done.history_id }));
            assert_eq!(s.hash(), after, "redo of {what}");
        }
    }
    assert!(edits >= 8, "non-trivial: only {edits} edit examples ran");
}

fn check_effect(effect: &Effect, before: &Scene, after: &Scene, what: &str) {
    let expected = reference::effect(before, after);
    assert_eq!(effect.parts, expected.parts, "{what}: changed parts");
    assert_eq!(
        effect.topology_changed, expected.topology_changed,
        "{what}: topology"
    );
    assert_eq!(
        effect.verts_touched, expected.verts_touched,
        "{what}: vertices touched"
    );
    let bbox = effect.bbox.map(|[min, max]| [min.0, max.0]);
    assert_eq!(
        bits(bbox),
        bits(expected.bbox),
        "{what}: bounds of the touched vertices"
    );
    assert!(
        effect.noop_reason.is_none(),
        "{what}: a real change has no no-op reason"
    );
    // A part placed anew moves every scene point with it, so the scene cannot tell whether its
    // own points moved too: neither side names its positions.
    let placed: BTreeSet<&String> = (effect.changed.iter())
        .filter(|(_, names)| names.iter().any(|n| n == "transform"))
        .map(|(part, _)| part)
        .collect();
    let named = (effect.changed.iter()).map(|(part, names)| (part.clone(), names.iter().cloned()));
    assert_eq!(
        shown(named.map(|(p, n)| (p, n.collect())).collect(), &placed),
        shown(names_shown(before, after), &placed),
        "{what}: the names of what changed"
    );
}

/// `changed` as far as the scene shows it: no properties or levels, and no positions for the
/// `placed` parts.
fn shown(
    changed: BTreeMap<String, BTreeSet<String>>,
    placed: &BTreeSet<&String>,
) -> BTreeMap<String, BTreeSet<String>> {
    let hidden = [
        "transform",
        "rank",
        "visible",
        "paint_visible",
        "voxel_mode",
        "operation",
        "start",
        "levels",
    ];
    (changed.into_iter())
        .map(|(part, mut names)| {
            names.retain(|n| !hidden.contains(&n.as_str()));
            if placed.contains(&part) {
                names.remove("positions");
            }
            (part, names)
        })
        .filter(|(_, names)| !names.is_empty())
        .collect()
}

/// What changed in each part from `before` to `after`, read slowly from the whole scene in the
/// names an effect gives: `positions`, `faces` and each column whose values or shape changed.
/// A part added or removed, or whose faces changed, names all of them on either side.
fn names_shown(before: &Scene, after: &Scene) -> BTreeMap<String, BTreeSet<String>> {
    let parts: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    let words = |m: &PlainMesh| {
        m.positions
            .iter()
            .map(|p| p.map(f32::to_bits))
            .collect::<Vec<_>>()
    };
    let mut changed = BTreeMap::new();
    for part in parts {
        let (old, new) = (before.get(part), after.get(part));
        let all: BTreeSet<String> = columns(old).chain(columns(new)).cloned().collect();
        let names: BTreeSet<String> = match (old, new) {
            (Some(a), Some(b)) if a.faces == b.faces && a.positions.len() == b.positions.len() => {
                let differ =
                    |c: &String| column(a.attributes.get(c)) != column(b.attributes.get(c));
                let moved = (words(a) != words(b)).then(|| "positions".to_string());
                all.into_iter().filter(differ).chain(moved).collect()
            }
            _ => (all.into_iter())
                .chain(["positions", "faces"].map(str::to_string))
                .collect(),
        };
        if !names.is_empty() {
            changed.insert(part.clone(), names);
        }
    }
    changed
}

/// The names of the columns of `m`, if there is one.
fn columns(m: Option<&PlainMesh>) -> impl Iterator<Item = &String> {
    m.into_iter().flat_map(|m| m.attributes.keys())
}

/// A column as plain bits: its domain, its value type and its values.
fn column(c: Option<&Attribute>) -> Option<(String, u8, Vec<u32>)> {
    c.map(|c| {
        let (kind, values) = match &c.values {
            AttrValues::F32(v) => (0, v.iter().map(|x| x.to_bits()).collect()),
            AttrValues::U32(v) => (1, v.clone()),
        };
        (format!("{:?}", c.domain), kind, values)
    })
}

fn bits(bbox: Option<[[f32; 3]; 2]>) -> Option<[[u32; 3]; 2]> {
    bbox.map(|corners| corners.map(|c| c.map(f32::to_bits)))
}

#[test]
fn search_finds_every_command_by_its_name_first() {
    let mut s = agent();
    for info in catalog().commands {
        let found = s.query(
            "commands.search",
            json!({ "query": info.name, "limit": 1000 }),
        );
        assert_eq!(
            found["matches"][0]["name"],
            json!(info.name),
            "searching for {}",
            info.name
        );
        assert_eq!(
            found["matches"][0]["summary"],
            json!(info.summary),
            "{} summary",
            info.name
        );
        assert_eq!(
            found["matches"][0]["kind"],
            json!(info.kind),
            "{} kind",
            info.name
        );
    }
}

#[test]
fn search_ranks_name_matches_before_docs_matches_and_respects_the_limit() {
    let mut s = agent();
    let names = |found: &Value| -> Vec<String> {
        found["matches"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|m| text(&m["name"]))
            .collect()
    };
    let found = names(&s.query("commands.search", json!({ "query": "Part", "limit": 1000 })));
    let in_name = found.iter().take_while(|n| n.contains("part")).count();
    assert!(in_name >= 2, "commands named part.* come first: {found:?}");
    assert!(
        found.len() > in_name,
        "commands that only mention parts follow: {found:?}"
    );
    assert!(
        found[in_name..].iter().all(|n| !n.contains("part")),
        "{found:?}"
    );
    let mut sorted = found[in_name..].to_vec();
    sorted.sort();
    assert_eq!(
        found[in_name..],
        sorted[..],
        "docs matches are sorted by name"
    );
    let one = names(&s.query("commands.search", json!({ "query": "part", "limit": 1 })));
    assert_eq!(one, found[..1], "limit 1");
    let none = names(&s.query("commands.search", json!({ "query": "zzqx" })));
    assert!(none.is_empty(), "nothing matches zzqx: {none:?}");
}

#[test]
fn describe_returns_the_catalog_entry_and_suggests_close_names() {
    let mut s = agent();
    for info in catalog().commands {
        let described = s.query("commands.describe", json!({ "name": info.name }));
        assert_eq!(described, json!(info), "describe {}", info.name);
    }
    let err = s.err("commands.describe", json!({ "name": "primitive.sphre" }));
    assert_eq!(code(err.code), "E_NOT_FOUND", "unknown name");
    assert_eq!(
        err.suggestions.first().map(String::as_str),
        Some("primitive.sphere"),
        "did you mean"
    );
}

#[test]
fn unknown_commands_get_close_suggestions() {
    let mut s = agent();
    let err = s.err("primitive.sphre", json!({}));
    assert_eq!(code(err.code), "E_UNKNOWN_COMMAND", "misspelled name");
    assert_eq!(err.suggestions[0], "primitive.sphere", "closest name first");
    assert!(err.suggestions.len() <= 3, "a few candidates");
    let err = s.err("qqqqzzzz", json!({}));
    assert!(
        err.suggestions.is_empty(),
        "nothing resembles qqqqzzzz: {:?}",
        err.suggestions
    );
    let err = s.err("primitive.sphere", json!({ "qqqqzzzz": 1 }));
    assert!(
        err.suggestions.is_empty(),
        "no argument resembles qqqqzzzz: {:?}",
        err.suggestions
    );
    let err = s.err("undo", json!({}));
    assert_eq!(
        err.suggestions[0], "history.undo",
        "a bare verb finds its command"
    );
    assert!(err.hint.is_some(), "says how to find commands");
}

#[test]
fn argument_errors_report_every_violation_with_help() {
    let mut s = agent();
    let err = s.err(
        "primitive.sphere",
        json!({ "subdivisions": 0, "radius": -1, "center": [1] }),
    );
    let paths: Vec<&str> = err.violations.iter().map(|v| v.path.as_str()).collect();
    for expected in ["/subdivisions", "/radius", "/center"] {
        assert!(
            paths.contains(&expected),
            "{expected} missing from {paths:?}"
        );
    }
    assert_eq!(
        err.path.as_deref(),
        paths.first().copied(),
        "the path is the first violation"
    );
    assert!(
        err.message.contains(&err.violations[0].message),
        "message: {}",
        err.message
    );
    let sphere = catalog()
        .commands
        .into_iter()
        .find(|c| c.name == "primitive.sphere");
    let example = sphere.and_then(|c| c.examples.first().map(|e| e.args.clone()));
    assert_eq!(err.example, example, "a valid example comes with the error");
    assert!(
        err.hint
            .as_deref()
            .is_some_and(|h| h.contains("primitive.sphere")),
        "hint names the command"
    );

    let err = s.err("primitive.sphere", json!({ "subdivisons": 3 }));
    assert_eq!(
        err.suggestions[0], "subdivisions",
        "did you mean, for a misspelled argument"
    );
    let mesh = json!({ "positions": [], "faces": [], "facez": [] });
    let err = s.err("part.create", json!({ "name": "x", "mesh": mesh }));
    assert_eq!(
        err.path.as_deref(),
        Some("/mesh/facez"),
        "nested unknown field"
    );
    assert_eq!(
        err.suggestions[0], "faces",
        "did you mean, inside a nested object"
    );
    let err = s.err(
        "mesh.hash",
        json!({ "part": "x", "mesh": { "positions": [], "faces": [] } }),
    );
    let message = &err.violations[0].message;
    assert!(
        message.contains("part") && message.contains("mesh"),
        "names both: {message}"
    );
    assert!(
        !message.contains("positions"),
        "and does not echo the arguments: {message}"
    );
}

#[test]
fn values_the_schema_allows_but_the_decoder_refuses_are_argument_errors() {
    let mut s = agent();
    s.ok("primitive.cube", json!({}));
    let huge = format!("1{}%", "0".repeat(40));
    let err = s.err(
        "mesh.translate",
        json!({ "direction": [1, 0, 0], "distance": huge }),
    );
    assert_eq!(
        code(err.code),
        "E_ARGS_INVALID",
        "an overflowing percentage"
    );
    assert_eq!(
        err.path.as_deref(),
        Some("/distance"),
        "points at the value"
    );
    assert!(err.example.is_some() && err.hint.is_some(), "with help");
    assert!(
        err.message.starts_with("mesh.translate: "),
        "names the command: {}",
        err.message
    );
}

#[test]
fn the_catalog_is_stable_and_described_by_session_info() {
    let first = catalog();
    let second = catalog();
    assert_eq!(first.hash, second.hash, "same commands, same hash");
    assert_eq!(first.hash.len(), 64, "a full BLAKE3 hash");
    let info = agent().query("session.info", json!({}));
    assert_eq!(
        info["catalog_hash"],
        json!(first.hash),
        "session.info names the catalog"
    );
    assert_eq!(
        info["commands"],
        json!(first.commands.len()),
        "and counts it"
    );
    assert_eq!(
        info["engine_version"],
        json!(first.engine_version),
        "same version"
    );
}

/// The JSON types a schema allows, following `anyOf` branches.
fn types(schema: &Value) -> Vec<String> {
    let own = match &schema["type"] {
        Value::String(t) => vec![t.clone()],
        Value::Array(ts) => ts
            .iter()
            .filter_map(|t| t.as_str().map(String::from))
            .collect(),
        _ => Vec::new(),
    };
    let branches = schema["anyOf"].as_array().into_iter().flatten();
    own.into_iter().chain(branches.flat_map(types)).collect()
}

/// Every argument the engine fills in for a command exists in its schema with the right type:
/// `part` refers to a part, `name` is text, `seed` is a whole number, `view` names a view,
/// `viewport` is a pair of whole numbers, `pivot` a point and `camera` a view spec. A command
/// whose flag names a field it does not have would otherwise get an empty part name or a seed
/// it ignores.
#[test]
fn pinned_arguments_match_the_schemas() {
    let mut pinned = 0;
    for info in catalog().commands {
        for arg in &info.pinned {
            let schema = info
                .args_schema
                .pointer(&format!("/properties/{arg}"))
                .unwrap_or_else(|| panic!("{} pins '{arg}', which it does not have", info.name));
            let fits = match arg.as_str() {
                "part" => schema.to_string().contains("#/$defs/PartRef"),
                "name" => types(schema).contains(&"string".to_string()),
                "seed" => types(schema).contains(&"integer".to_string()),
                "view" => schema.to_string().contains("#/$defs/ViewName"),
                "viewport" => types(schema).contains(&"array".to_string()),
                "pivot" | "point" => schema.to_string().contains("#/$defs/Vec3"),
                "camera" => schema.to_string().contains("#/$defs/ViewSpec"),
                other => panic!("{}: unknown pinned argument '{other}'", info.name),
            };
            assert!(fits, "{}: '{arg}' has the wrong type: {schema}", info.name);
            pinned += 1;
        }
    }
    assert!(pinned >= 8, "non-trivial: only {pinned} pinned arguments");
}

/// The catalog says which commands only the human may run, so clients can hide or label them.
#[test]
fn the_catalog_marks_human_only_commands() {
    let human_only: Vec<String> = catalog()
        .commands
        .into_iter()
        .filter(|c| c.human_only)
        .map(|c| c.name)
        .collect();
    assert_eq!(
        human_only,
        ["gizmo.deformer", "gizmo.drag", "session.control"],
        "human-only commands"
    );
}
