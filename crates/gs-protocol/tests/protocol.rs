//! Protocol properties: the `Length` parser agrees with the schema clients see, lengths survive
//! a round trip through JSON bit for bit, sockets and saved state live where the docs say, and
//! a face's corners are read, and a quad split, one way.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::Shutdown;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gs_protocol::{
    Env, Length, NO_VERTEX, RpcRequest, RunParams, ScriptLine, System, UnixListener, UnixStream,
    arity, cache_dir, cache_dir_in, check_session_name, check_socket_path, config_dir,
    config_dir_in, corners, face_triangles, free_path, rpc_code, session_dir, session_dir_in,
    socket_path, socket_path_in, state_dir, state_dir_in,
};

#[test]
fn a_face_has_three_corners_with_the_marker_in_its_fourth_slot_and_four_without() {
    // What every reader of faces relies on: a triangle's three corners and a quad's four, in
    // winding order (welding compares faces by them, splitting walks their edges).
    let (triangle, quad) = ([4, 5, 6, NO_VERTEX], [4, 5, 6, 7]);
    assert_eq!((arity(triangle), corners(&triangle)), (3, &[4, 5, 6][..]));
    assert_eq!((arity(quad), corners(&quad)), (4, &[4, 5, 6, 7][..]));
    // Only the fourth slot decides, so a marker in an earlier one stays among the corners, where
    // the validators report it as misplaced. Welding and splitting never meet such a face: a
    // mesh store refuses it.
    let marked = [4, NO_VERTEX, 6, 7];
    assert_eq!(corners(&marked), [4, NO_VERTEX, 6, 7]);
}

/// `face` turned to start at its corner `turn`.
fn turned(face: [u32; 4], turn: usize) -> [u32; 4] {
    std::array::from_fn(|i| face[(i + turn) % 4])
}

/// The triangles `face_triangles` splits `face` into, with vertex `v` at `pos(v)`, as sorted
/// vertex triples, sorted: the split itself, whichever corner the face starts at.
fn split_of(face: [u32; 4], pos: &dyn Fn(u32) -> [f32; 3]) -> Vec<[u32; 3]> {
    let mut tris: Vec<[u32; 3]> = (face_triangles(face, |k| pos(face[k])).iter())
        .map(|t| {
            let mut t = t.map(|c| face[c]);
            t.sort_unstable();
            t
        })
        .collect();
    tris.sort_unstable();
    tris
}

/// The tie key's signed half (decision 0040, section 2). Each side of a cube of one face a side
/// has four corners alike by `|x|`, `|y|` and `|z|`, so by those alone the corner a face starts
/// at would pick its diagonal. With the signs, the x = 1 side splits through the same corners
/// whichever corner it starts at, and like its mirror image, the x = -1 side.
#[test]
fn a_square_whose_corners_tie_by_size_splits_by_their_signs() {
    let p = [
        [1.0, -1.0, -1.0],
        [1.0, 1.0, -1.0],
        [1.0, 1.0, 1.0],
        [1.0, -1.0, 1.0],
    ];
    let pos = |v: u32| p[v as usize];
    let image = |v: u32| {
        let [x, y, z] = p[v as usize];
        [-x, y, z]
    };
    // Corner 0 comes first by y, then z, so the split is along 0-2 on both sides.
    let along_0_2 = vec![[0, 1, 2], [0, 2, 3]];
    for turn in 0..4 {
        assert_eq!(
            split_of(turned([0, 1, 2, 3], turn), &pos),
            along_0_2,
            "turned {turn}"
        );
        let mirrored = turned([0, 3, 2, 1], turn);
        assert_eq!(
            split_of(mirrored, &image),
            along_0_2,
            "the image, turned {turn}"
        );
    }
}

/// A triangle is drawn, picked and measured whole, and its positions are not read.
#[test]
fn a_triangle_is_left_whole() {
    let pos = |_: usize| -> [f32; 3] { panic!("a triangle's positions were read") };
    assert_eq!(face_triangles([4, 5, 6, NO_VERTEX], pos), [[0, 1, 2]]);
}

use jsonschema::Validator;
use proptest::prelude::*;
use serde_json::{Value, json};

fn length_schema() -> Validator {
    let schema = schemars::schema_for!(Length);
    jsonschema::validator_for(schema.as_value())
        .unwrap_or_else(|e| panic!("the Length schema does not compile: {e}"))
}

fn parses(value: &Value) -> bool {
    serde_json::from_value::<Length>(value.clone()).is_ok()
}

/// Every string of up to four characters from an alphabet that covers the grammar's edges.
fn short_strings() -> Vec<String> {
    let alphabet = ['0', '7', '.', '+', '-', '%', 'e', ' '];
    let mut all = vec![String::new()];
    let mut layer = vec![String::new()];
    for _ in 0..4 {
        layer = layer
            .iter()
            .flat_map(|s| alphabet.iter().map(move |c| format!("{s}{c}")))
            .collect();
        all.extend(layer.iter().cloned());
    }
    all
}

/// Differential: the hand-written parser and the schema's regex are independent definitions of
/// the same rule, so on every short string (none can overflow) they must agree.
#[test]
fn length_parser_agrees_with_the_schema_on_every_short_string() {
    let schema = length_schema();
    let strings = short_strings();
    let accepted = strings.iter().filter(|s| parses(&json!(s))).count();
    assert!(
        accepted > 20,
        "non-trivial: many short strings are lengths, got {accepted}"
    );
    for text in strings {
        let value = json!(text);
        assert_eq!(
            parses(&value),
            schema.is_valid(&value),
            "parser and schema disagree on {text:?}"
        );
    }
}

#[test]
fn lengths_that_are_not_finite_are_refused() {
    let schema = length_schema();
    let huge_percent = json!(format!("1{}%", "0".repeat(40)));
    assert!(
        schema.is_valid(&huge_percent),
        "the pattern allows any digit string"
    );
    assert!(
        !parses(&huge_percent),
        "a percentage too big for f32 is refused"
    );
    let huge_number: Value = serde_json::from_str("1e39").expect("valid JSON");
    assert!(!parses(&huge_number), "a number too big for f32 is refused");
    assert!(
        parses(&json!(3.0e38)),
        "the largest finite magnitudes are fine"
    );
}

fn finite_f32() -> impl Strategy<Value = f32> {
    prop_oneof![
        proptest::num::f32::NORMAL | proptest::num::f32::SUBNORMAL | proptest::num::f32::ZERO,
        (-1000i32..1000).prop_map(|n| n as f32 / 8.0),
    ]
}

proptest! {
    /// Round trip: a length written to JSON validates against its schema and reads back to
    /// the same bits.
    #[test]
    fn lengths_round_trip_bit_for_bit(value in finite_f32(), percent in any::<bool>()) {
        let length = if percent { Length::Percent(value) } else { Length::Units(value) };
        let written = serde_json::to_value(length).expect("serializes");
        prop_assert!(length_schema().is_valid(&written), "{written} fails the schema");
        prop_assert_eq!(written.is_string(), percent, "percentages are strings, units are numbers");
        let read: Length = serde_json::from_value(written.clone()).expect("reads back");
        let bits = |l: Length| match l {
            Length::Units(v) => (false, v.to_bits()),
            Length::Percent(v) => (true, v.to_bits()),
        };
        prop_assert_eq!(bits(read), bits(length), "{} changed in a round trip", written);
    }

    /// Decision 0040, section 2 (registry property): a quad and its mirror image across x, y
    /// or z = 0 split along mirror-image diagonals whichever corner each face starts at, along
    /// the shorter diagonal, into the two triangles that keep the face's winding. Corners on a
    /// small grid tie often, so the corner key decides many quads; no coordinate is negative,
    /// so no quad crosses the plane (no rule splits such a quad like its image).
    #[test]
    fn mirror_image_quads_split_alike_whatever_corner_they_start_at(
        grid in prop::array::uniform4(prop::array::uniform3(0u8..4)),
        axis in 0usize..3,
        turns in (0usize..4, 0usize..4),
    ) {
        prop_assume!((0..4).all(|i| (i + 1..4).all(|j| grid[i] != grid[j])), "distinct corners");
        let p = grid.map(|c| c.map(f32::from));
        let pos = |v: u32| p[v as usize];
        let image = |v: u32| {
            let mut q = p[v as usize];
            q[axis] = -q[axis];
            q
        };
        let face = turned([0, 1, 2, 3], turns.0);
        let split = split_of(face, &pos);
        // Mirroring reverses the winding.
        prop_assert_eq!(&split, &split_of(turned([0, 3, 2, 1], turns.1), &image), "{:?}", p);
        // Squared lengths, exact on the grid.
        let length =
            |a: usize, b: usize| (0..3).map(|i| (i32::from(grid[a][i]) - i32::from(grid[b][i])).pow(2)).sum::<i32>();
        let diagonal: Vec<u32> = split[0].iter().filter(|v| split[1].contains(v)).copied().collect();
        if length(0, 2) != length(1, 3) {
            let shorter = if length(0, 2) < length(1, 3) { [0, 2] } else { [1, 3] };
            prop_assert_eq!(&diagonal[..], &shorter[..], "{:?}", p);
        }
        let by_corner = face_triangles(face, |k| pos(face[k]));
        prop_assert!(
            by_corner == [[0, 1, 2], [0, 2, 3]] || by_corner == [[1, 2, 3], [1, 3, 0]],
            "{by_corner:?}"
        );
    }
}

/// A made-up absolute folder: `/` then `path` (`C:\` then `path` on Windows, where a path needs a
/// drive to be absolute).
fn root(path: &str) -> PathBuf {
    PathBuf::from(if cfg!(windows) { r"C:\" } else { "/" }).join(path)
}

/// A made-up environment holding only `vars`.
fn env(vars: &[(&str, PathBuf)]) -> impl Fn(&str) -> Option<OsString> + use<> {
    let vars: Vec<(String, OsString)> = (vars.iter())
        .map(|(name, value)| ((*name).to_string(), value.clone().into_os_string()))
        .collect();
    move |name| (vars.iter().find(|(n, _)| n == name)).map(|(_, value)| value.clone())
}

const SYSTEMS: [System; 3] = [System::Linux, System::MacOs, System::Windows];

/// Clients and the engine find sessions without talking first, so the session folder is a
/// contract: `$XDG_RUNTIME_DIR/gigasculpt` on every system when that is set (tests and sessions
/// run side by side keep apart through it); without it none on Linux, and the `sessions` folder
/// in the app's own folder on macOS and Windows. A socket is `<name>.sock` there.
#[test]
fn sockets_live_in_the_session_folder() {
    let (home, local) = (("HOME", root("h/u")), ("LOCALAPPDATA", root("l")));
    for system in SYSTEMS {
        let set = env(&[("XDG_RUNTIME_DIR", root("r")), home.clone(), local.clone()]);
        assert_eq!(
            session_dir_in(system, &set),
            Ok(root("r").join("gigasculpt")),
            "{system:?}: XDG_RUNTIME_DIR wins"
        );
    }
    let unset = env(&[home, local]);
    assert!(
        session_dir_in(System::Linux, &unset).is_err(),
        "on Linux there is no session folder without XDG_RUNTIME_DIR"
    );
    assert_eq!(
        session_dir_in(System::MacOs, &unset),
        Ok(root("h/u/Library/Application Support/GigaSculpt/sessions")),
        "macOS"
    );
    assert_eq!(
        session_dir_in(System::Windows, &unset),
        Ok(root("l/GigaSculpt/sessions")),
        "Windows"
    );
    assert_eq!(
        session_dir(),
        session_dir_in(System::HERE, &|name| std::env::var_os(name)),
        "session_dir reads this system's environment"
    );
    assert_eq!(
        socket_path("t-1_x"),
        session_dir().map(|dir| dir.join("t-1_x.sock")),
        "a socket is <name>.sock in the session folder"
    );
}

/// A socket's path fits in its system's `sun_path` less the closing NUL: 107 bytes on Linux and
/// Windows, 103 on macOS. A longer session name is refused before any bind, naming the limit and
/// the longest name that fits in that session folder.
#[test]
fn a_session_name_too_long_for_its_socket_is_refused() {
    let vars = env(&[("XDG_RUNTIME_DIR", root("run/user/1000"))]);
    for (system, limit) in [
        (System::Linux, 107),
        (System::MacOs, 103),
        (System::Windows, 107),
    ] {
        let dir = session_dir_in(system, &vars).expect("a session folder");
        // The folder, a separator, the name and ".sock".
        let longest = limit - dir.as_os_str().len() - 6;
        let fits = "n".repeat(longest);
        assert_eq!(
            socket_path_in(system, &vars, &fits).map(|path| path.as_os_str().len()),
            Ok(limit),
            "{system:?}: the longest name fits exactly"
        );
        let refused = socket_path_in(system, &vars, &format!("{fits}n"))
            .expect_err("one character more is refused");
        assert!(
            refused.contains(&format!("at most {limit} bytes"))
                && refused.contains(&format!("up to {longest} characters")),
            "{system:?}: the refusal names the limit and the longest name: {refused}"
        );
        // A socket path given as is (`--socket`) is held to the same limit.
        let given = dir.join(format!("{fits}.sock"));
        assert_eq!(check_socket_path(system, &given), Ok(()), "{system:?}");
        let refused = check_socket_path(system, &dir.join(format!("{fits}n.sock")))
            .expect_err("one byte more is refused");
        assert!(
            refused.contains(&format!(
                "{} bytes, and sockets take at most {limit}",
                limit + 1
            )),
            "{system:?}: the refusal names the length and the limit: {refused}"
        );
    }
    assert_eq!(
        socket_path("x"),
        socket_path_in(System::HERE, &|name| std::env::var_os(name), "x"),
        "socket_path reads this system's environment"
    );
}

/// Autosaves, crash markers and command logs must survive a reboot, so they live in the state
/// folder (decision 0016, section 14): `$XDG_STATE_HOME/gigasculpt` on every system when that is
/// an absolute path; else on Linux, by the XDG Base Directory spec, `$HOME/.local/state`, and
/// elsewhere the app's own folder.
#[test]
fn state_lives_in_the_state_folder() {
    let (home, local) = (("HOME", root("h/u")), ("LOCALAPPDATA", root("l")));
    for system in SYSTEMS {
        let set = env(&[("XDG_STATE_HOME", root("s")), home.clone(), local.clone()]);
        assert_eq!(
            state_dir_in(system, &set),
            Ok(root("s").join("gigasculpt")),
            "{system:?}: XDG_STATE_HOME wins"
        );
    }
    let own = [
        (System::Linux, root("h/u/.local/state/gigasculpt")),
        (
            System::MacOs,
            root("h/u/Library/Application Support/GigaSculpt"),
        ),
        (System::Windows, root("l/GigaSculpt")),
    ];
    for (system, folder) in own {
        for ignored in ["", "relative/state"] {
            let vars = [
                ("XDG_STATE_HOME", ignored.into()),
                home.clone(),
                local.clone(),
            ];
            assert_eq!(
                state_dir_in(system, &env(&vars)),
                Ok(folder.clone()),
                "{system:?}: XDG_STATE_HOME {ignored:?} counts as unset"
            );
        }
        assert!(
            state_dir_in(system, &env(&[])).is_err(),
            "{system:?}: with none of its variables there is no state folder"
        );
    }
    assert_eq!(
        state_dir(),
        state_dir_in(System::HERE, &|name| std::env::var_os(name)),
        "state_dir reads this system's environment"
    );
}

/// What can be made again (the Blender exchange, the Quad Remesh program) goes in the cache
/// folder and the user's settings in the config folder: `$XDG_CACHE_HOME/gigasculpt` and
/// `$XDG_CONFIG_HOME/gigasculpt` on every system when that is an absolute path; else each
/// system's own place.
#[test]
fn the_cache_and_config_folders_are_where_each_system_keeps_them() {
    type Find = fn(System, Env) -> Result<PathBuf, String>;
    let vars = [
        ("HOME", root("h/u")),
        ("LOCALAPPDATA", root("l")),
        ("APPDATA", root("a")),
    ];
    let kinds: [(&str, Find, [PathBuf; 3]); 2] = [
        (
            "XDG_CACHE_HOME",
            cache_dir_in,
            [
                root("h/u/.cache/gigasculpt"),
                root("h/u/Library/Caches/GigaSculpt"),
                root("l/GigaSculpt/cache"),
            ],
        ),
        (
            "XDG_CONFIG_HOME",
            config_dir_in,
            [
                root("h/u/.config/gigasculpt"),
                root("h/u/Library/Application Support/GigaSculpt"),
                root("a/GigaSculpt"),
            ],
        ),
    ];
    for (var, find, own) in kinds {
        for (system, folder) in SYSTEMS.into_iter().zip(own) {
            let with = |value: PathBuf| env(&[vars.as_slice(), &[(var, value)]].concat());
            assert_eq!(
                (
                    find(system, &with(root("x"))),
                    find(system, &with("x".into()))
                ),
                (Ok(root("x").join("gigasculpt")), Ok(folder)),
                "{system:?}: {var} wins, a relative one counting as unset"
            );
            assert!(
                find(system, &env(&[])).is_err(),
                "{system:?}: with none of its variables there is no folder"
            );
        }
    }
    let here = |name: &str| std::env::var_os(name);
    assert_eq!(cache_dir(), cache_dir_in(System::HERE, &here), "cache_dir");
    assert_eq!(
        config_dir(),
        config_dir_in(System::HERE, &here),
        "config_dir"
    );
}

/// A session name is letters, digits, '-' and '_', so it can never reach outside the session
/// folder.
#[test]
fn session_names_stay_inside_the_session_folder() {
    for good in ["a", "main", "T-1_x"] {
        assert!(check_session_name(good).is_ok(), "{good:?} is a name");
    }
    for bad in ["", "a/b", "../x", "a b", "ü", "a.b", ".."] {
        assert!(check_session_name(bad).is_err(), "{bad:?} is not a name");
        assert!(socket_path(bad).is_err(), "{bad:?} has no socket");
    }
}

/// A request without an id is a notification; an explicit `null` id is still a request.
#[test]
fn requests_tell_a_missing_id_from_a_null_one() {
    let parse = |line: &str| {
        serde_json::from_str::<RpcRequest>(line)
            .unwrap_or_else(|e| panic!("{line} does not parse: {e}"))
            .id
    };
    let method = r#""jsonrpc": "2.0", "method": "catalog.get""#;
    assert_eq!(
        parse(&format!("{{{method}}}")),
        None,
        "no id: a notification"
    );
    assert_eq!(
        parse(&format!("{{{method}, \"id\": null}}")),
        Some(Value::Null),
        "a null id"
    );
    assert_eq!(
        parse(&format!("{{{method}, \"id\": 7}}")),
        Some(json!(7)),
        "an id"
    );
    let missing = RpcRequest {
        jsonrpc: "2.0".into(),
        id: None,
        method: "catalog.get".into(),
        params: Value::Null,
    };
    assert!(
        json!(missing).get("id").is_none(),
        "a notification is written without an id"
    );
}

/// A run request built in code is the call a client sends with only a command and its
/// arguments (no revision, not a dry run), and its dry run that call with `dry_run` set.
#[test]
fn a_run_request_built_in_code_is_the_call_a_client_sends_with_the_wire_s_defaults() {
    let sent = |call: Value| {
        let params: RunParams = serde_json::from_value(call.clone())
            .unwrap_or_else(|e| panic!("{call} does not parse: {e}"));
        json!(params)
    };
    let args = json!({ "part": "a" });
    assert_eq!(
        json!(RunParams::new("mesh.stats", args.clone())),
        sent(json!({ "command": "mesh.stats", "args": args })),
        "a run"
    );
    assert_eq!(
        json!(RunParams::new("mesh.stats", args.clone()).dry()),
        sent(json!({ "command": "mesh.stats", "args": args, "dry_run": true })),
        "a dry run"
    );
}

/// A log line becomes the request that replays it: a batch line one batch, any other line one
/// command.
#[test]
fn log_lines_replay_as_the_call_they_record() {
    let line = |text: &str| match ScriptLine::parse(text) {
        Ok(ScriptLine::Record(record)) => record,
        other => panic!("{text}: {other:?}"),
    };
    let single = line(r#"{"seq": 3, "command": "mesh.stats", "args": {"part": "a"}}"#);
    let request = single.request().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(request.method(), "cmd.run", "one command");
    assert_eq!(
        request.params(),
        json!({ "command": "mesh.stats", "args": { "part": "a" }, "dry_run": false }),
        "with its arguments"
    );
    let batch = line(
        r#"{"command": "cmd.batch", "args": {"commands": [{"command": "primitive.cube"}, {"command": "mesh.stats"}]}}"#,
    );
    let request = batch.request().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(request.method(), "cmd.batch", "a batch");
    assert_eq!(
        request.params()["commands"][1]["command"],
        json!("mesh.stats"),
        "with its steps"
    );
    let broken = line(r#"{"command": "cmd.batch", "args": {"steps": []}}"#);
    assert!(broken.request().is_err(), "a batch line needs its commands");
    assert!(
        ScriptLine::parse(r#"{"command": "mesh.stats", "arg": {}}"#).is_err(),
        "a misspelled field is refused"
    );
    let header = r#"{"gigasculpt_log": 1, "engine_version": "0", "protocol": 1, "catalog_hash": "x", "session": "s", "seed": 0, "verify": false}"#;
    assert!(
        matches!(ScriptLine::parse(header), Ok(ScriptLine::Header(_))),
        "a header"
    );
}

/// The engine's own error codes sit in the range JSON-RPC 2.0 sets aside for server errors
/// (-32099 to -32000), so a client can tell them from the standard codes.
#[test]
fn server_error_codes_are_in_the_server_range() {
    for code in [rpc_code::REFUSED, rpc_code::TOO_LARGE] {
        assert!((-32099..=-32000).contains(&code), "{code}");
    }
    assert_ne!(rpc_code::REFUSED, rpc_code::TOO_LARGE, "distinct");
}

/// One rule names every file that would land on another (review 0008): the path itself while it
/// is free, then `-1`, `-2` before the extension, for files and folders alike.
#[test]
fn a_taken_path_is_numbered_on() {
    let dir = std::env::temp_dir().join(format!("gs-free-path-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("session.jsonl");
    assert_eq!(free_path(&log), log);
    std::fs::write(&log, "").unwrap();
    assert_eq!(free_path(&log), dir.join("session-1.jsonl"));
    std::fs::write(dir.join("session-1.jsonl"), "").unwrap();
    assert_eq!(free_path(&log), dir.join("session-2.jsonl"));
    let bundle = dir.join("session-panic-7.gsrepro");
    std::fs::create_dir(&bundle).unwrap();
    assert_eq!(free_path(&bundle), dir.join("session-panic-7-1.gsrepro"));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The session socket works the same way on every system: a client connects to a `.sock` file,
/// sends a line, and reads the answer the server wrote through its stream while a clone of that
/// stream read the request. A read that waits past the stream's read timeout gives up with
/// `WouldBlock` or `TimedOut` and keeps what came before it, so a line split by a longer pause
/// arrives whole. Then the server stops as a session does (`Shared::stopping`): with the client
/// still connected, it sets a flag and shuts the read side of a third clone, and the thread
/// waiting for the next line wakes at once, or at its next timeout where the shutdown does not
/// reach a read already waiting (Windows). The shutdown comes while both ends are open, as the
/// engine's does: macOS refuses one on a socket whose other end has closed ("not connected"),
/// where Linux does not.
#[test]
fn a_session_socket_carries_a_line_each_way() {
    const CHECK: Duration = Duration::from_millis(100);
    let dir = std::env::temp_dir().join(format!("gs-socket-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let ((closer, closing), (ended, end)) =
        (std::sync::mpsc::channel(), std::sync::mpsc::channel());
    let server = {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(CHECK)).unwrap();
            closer.send(stream.try_clone().unwrap()).unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let (mut line, mut waits) = (Vec::new(), 0);
            let read = until_stopped(&mut reader, &mut line, &stop, &mut waits);
            assert!(matches!(read, Ok(n) if n > 0), "the first line: {read:?}");
            (&stream).write_all(&line.to_ascii_uppercase()).unwrap();
            let paused = waits;
            line.clear();
            let woke = until_stopped(&mut reader, &mut line, &stop, &mut waits);
            ended.send((paused, woke, line)).unwrap();
        })
    };
    let mut client = UnixStream::connect(&path).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    client.write_all(b"pi").unwrap();
    std::thread::sleep(CHECK * 5);
    client.write_all(b"ng\n").unwrap();
    let mut reply = String::new();
    BufReader::new(&client).read_line(&mut reply).unwrap();
    assert_eq!(reply, "PING\n", "the line split by a pause arrives whole");
    stop.store(true, Ordering::SeqCst);
    closing.recv().unwrap().shutdown(Shutdown::Read).unwrap();
    let (paused, woke, rest) = end.recv_timeout(CHECK * 20).unwrap_or_else(|e| {
        panic!("the waiting thread wakes promptly at the stop, not later: {e}")
    });
    server.join().unwrap();
    assert!(paused > 0, "the pause outlasted the read timeout");
    assert!(
        rest.is_empty() && !matches!(woke, Ok(n) if n > 0),
        "nothing more was read: {woke:?}"
    );
    drop(client);
    std::fs::remove_dir_all(&dir).ok();
}

/// Reads one line into `line` as a session's connection does, counting each timed-out read in
/// `waits` and reading on after it until `stop` is set.
fn until_stopped(
    reader: &mut impl BufRead,
    line: &mut Vec<u8>,
    stop: &AtomicBool,
    waits: &mut u32,
) -> std::io::Result<usize> {
    loop {
        match reader.read_until(b'\n', line) {
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                *waits += 1;
                if stop.load(Ordering::SeqCst) {
                    return Err(e);
                }
            }
            read => return read,
        }
    }
}
