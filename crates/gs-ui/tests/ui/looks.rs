//! Offscreen looks (`view.snapshot`, `view.contact_sheet`) through gs-ui's render hosts: a
//! headless session's own device, and the window's device, whose camera never moves.

use gs_engine::{Engine, EngineConfig, Origin};
use gs_protocol::{CommandError, Outcome, Request, Role, RunParams};
use gs_render::{AdapterKind, Renderer};
use gs_ui::{Host, Keymap, Layout, LayoutFile, Offscreen, Render, Shell, Studio};
use serde_json::{Value, json};

/// A decoded picture: its width and RGBA rows.
pub type Shot = (u32, Vec<u8>);

/// Runs a command as an agent: its result, or its error.
fn call(engine: &mut Engine, command: &str, args: Value) -> Result<Value, Box<CommandError>> {
    let agent = Origin {
        client_id: 9,
        client: "agent".into(),
        role: Role::Agent,
    };
    let request = Request::Run(RunParams::new(command, args));
    match engine.execute(&agent, request).outcome {
        Outcome::Ok(ok) => Ok(ok.result),
        Outcome::Error(e) => Err(Box::new(e)),
    }
}

/// Runs a command that must succeed.
pub fn run(engine: &mut Engine, command: &str, args: Value) -> Value {
    call(engine, command, args).unwrap_or_else(|e| panic!("{command}: {}", e.message))
}

fn decode(png: &[u8]) -> Shot {
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().unwrap_or_else(|e| panic!("{e}"));
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or_default()];
    let info = reader
        .next_frame(&mut buf)
        .unwrap_or_else(|e| panic!("{e}"));
    buf.truncate(info.buffer_size());
    (info.width, buf)
}

fn base64_decode(text: &str) -> Vec<u8> {
    use base64::Engine as _;
    let engine = base64::engine::general_purpose::STANDARD;
    engine.decode(text).unwrap_or_else(|e| panic!("{e}"))
}

/// An engine in a window: a studio drawing on lavapipe, attached as the window and as the
/// renderer, as the app does.
fn windowed() -> Engine {
    window().0
}

/// [`windowed`], and the window.
fn window() -> (Engine, Host) {
    let mut engine = Engine::new(EngineConfig::default()).unwrap_or_else(|e| panic!("{e}"));
    let layout = LayoutFile::in_memory(Layout::default());
    let shell = Shell::new(engine.catalog().clone(), Keymap::default(), layout);
    let renderer = Renderer::new(AdapterKind::Lavapipe).unwrap_or_else(|e| panic!("{e}"));
    let render = Render::new(renderer, 1).unwrap_or_else(|e| panic!("{e}"));
    let host = Host::new(Studio::new(shell, Some(render), [640, 400], 1.0));
    engine.attach_window(Box::new(host.clone()));
    engine.attach_renderer(Box::new(host.clone()));
    (engine, host)
}

/// The background every pixel no surface covers has.
const BACKGROUND: [u8; 4] = [40, 42, 46, 255];

/// A sphere and, to its right, a box: a scene that differs from every side.
fn scene(engine: &mut Engine) {
    run(engine, "primitive.sphere", json!({ "subdivisions": 3 }));
    run(engine, "primitive.cube", json!({}));
    let right = json!({ "part": "cube", "direction": [1, 0, 0], "distance": 2.5 });
    run(engine, "mesh.translate", right);
}

/// A headless engine rendering on lavapipe, with the scene.
pub fn headless() -> Engine {
    let mut engine = Engine::new(EngineConfig::default()).unwrap_or_else(|e| panic!("{e}"));
    let render = Offscreen::host(Some("lavapipe")).unwrap_or_else(|e| panic!("{e}"));
    engine.attach_renderer(render);
    scene(&mut engine);
    engine
}

/// A look's result and its picture.
fn look(engine: &mut Engine, command: &str, args: Value) -> (Value, Shot) {
    let out = run(engine, command, args);
    let shot = picture(&out);
    (out, shot)
}

/// The picture a result's `png_base64` holds.
pub fn picture(out: &Value) -> Shot {
    let png = out["png_base64"].as_str().unwrap_or_default();
    decode(&base64_decode(png))
}

pub fn snapshot(engine: &mut Engine, args: Value) -> Shot {
    look(engine, "view.snapshot", args).1
}

fn height(shot: &Shot) -> u32 {
    (shot.1.len() / 4) as u32 / shot.0.max(1)
}

fn background(shot: &Shot) -> usize {
    shot.1.chunks(4).filter(|p| *p == BACKGROUND).count()
}

/// The rows `from..to` of the columns `x0..x0 + width`.
fn block(shot: &Shot, x0: u32, width: u32, rows: std::ops::Range<u32>) -> Vec<u8> {
    let row = |y: u32| {
        let start = ((y * shot.0 + x0) * 4) as usize;
        shot.1[start..start + (width * 4) as usize].to_vec()
    };
    rows.flat_map(row).collect()
}

#[test]
fn a_snapshot_is_one_png_of_the_size_asked() {
    let mut engine = headless();
    let (out, shot) = look(
        &mut engine,
        "view.snapshot",
        json!({ "view": "front", "size": 300 }),
    );
    assert_eq!((shot.0, height(&shot)), (300, 300), "the size asked");
    assert_eq!(
        (out["width"].clone(), out["height"].clone()),
        (json!(300), json!(300))
    );
    assert_eq!(out["views"], json!(["front"]), "what it shows");
    assert_eq!(out["shading"], json!("matcap"), "the default look");
    assert!(
        background(&shot) < 300 * 300,
        "the parts are in the picture"
    );
    let (out, shot) = look(&mut engine, "view.snapshot", json!({}));
    assert_eq!(out["views"], json!(["iso"]), "iso by default");
    assert_eq!((shot.0, height(&shot)), (1024, 1024), "1024 px by default");

    let file = std::env::temp_dir().join(format!("gs-look-{}.png", std::process::id()));
    let path = file.display().to_string();
    let args = json!({ "view": "front", "size": 300, "path": path });
    let out = run(&mut engine, "view.snapshot", args);
    let written = std::fs::read(&file).unwrap_or_default();
    std::fs::remove_file(&file).ok();
    assert_eq!(out["path"], json!(path), "{out}");
    assert!(out.get("png_base64").is_none(), "written, not returned");
    let front = snapshot(&mut engine, json!({ "view": "front", "size": 300 }));
    assert_eq!(decode(&written), front, "the same picture");
}

#[test]
fn views_cameras_and_shading_change_the_picture() {
    let mut engine = headless();
    let at = |e: &mut Engine, view: Value| snapshot(e, json!({ "view": view, "size": 128 }));
    let named: Vec<Shot> = ["front", "back", "left", "top", "bottom", "right", "iso"]
        .iter()
        .map(|name| at(&mut engine, json!(name)))
        .collect();
    for (i, a) in named.iter().enumerate() {
        for b in &named[i + 1..] {
            assert_ne!(a, b, "each named view sees the scene differently");
        }
    }
    let front = &named[0];
    let level = json!({ "azimuth_deg": 0, "elevation_deg": 0 });
    let (out, shot) = look(
        &mut engine,
        "view.snapshot",
        json!({ "view": level, "size": 128 }),
    );
    assert_eq!(&shot, front, "orbit angles 0, 0 are the front view");
    assert_eq!(
        out["views"],
        json!(["front az 0 el 0"]),
        "labelled by its angles"
    );
    let turned = json!({ "view": "right", "azimuth_deg": 0 });
    assert_eq!(
        &at(&mut engine, turned),
        front,
        "given angles replace the view's"
    );
    assert_eq!(
        at(&mut engine, json!({ "view": "back" })),
        named[1],
        "an orbit from a view"
    );
    let far = at(&mut engine, json!({ "distance": 100 }));
    assert!(
        background(&far) > background(front),
        "farther away, smaller"
    );
    assert_ne!(
        &at(&mut engine, json!({ "target": [2.5, 0, 0] })),
        front,
        "looks elsewhere"
    );

    let shaded = |e: &mut Engine, shading: &str, wireframe: bool| {
        let args =
            json!({ "view": "iso", "size": 128, "shading": shading, "wireframe": wireframe });
        snapshot(e, args)
    };
    let looks: Vec<Shot> = ["matcap", "clay", "zebra", "normals"]
        .iter()
        .map(|s| shaded(&mut engine, s, false))
        .collect();
    for (i, a) in looks.iter().enumerate() {
        for b in &looks[i + 1..] {
            assert_ne!(a, b, "each shading looks different");
        }
    }
    assert_ne!(
        shaded(&mut engine, "clay", true),
        looks[1],
        "the wireframe shows"
    );
}

/// A sphere with its right side hidden (Select Rect, as the golden `hidden-sphere`) and a
/// patch on its front masked.
fn hidden_and_masked(engine: &mut Engine) {
    run(engine, "primitive.sphere", json!({ "subdivisions": 16 }));
    let corners = [[-2.0, -2.0], [0.3, 2.0]];
    let samples: Vec<Value> = (corners.iter())
        .map(|[x, y]| json!({ "position": [x, y, 3.0] }))
        .collect();
    let hide = json!({ "brush": "Select Rect", "view_dir": [0, 0, -1], "samples": samples });
    run(engine, "brush.stroke", hide);
    let at = json!([{ "position": [-0.3, 0.2, 1] }]);
    let mask = json!({ "brush": "Mask Pen", "size": 0.5, "strength": 1, "samples": at });
    run(engine, "brush.stroke", mask);
}

#[test]
fn a_look_is_the_windows_frame_of_the_same_view_hidden_faces_and_mask_too() {
    let (mut engine, host) = window();
    // A layout with no panels: beside the trays, this 640-pixel window would leave the docked
    // reference too small a picture to compare.
    host.lock().shell.directions.list[0].panels.clear();
    hidden_and_masked(&mut engine);
    // A Reference View showing the scene live: the window draws it into a frame of its own,
    // nothing over it, from the window's GPU copies of the parts.
    run(&mut engine, "reference.open", json!({ "scene": true }));
    run(&mut engine, "ui.screenshot", json!({}));
    let (width, frame) = {
        let studio = host.lock();
        let render = studio
            .render
            .as_ref()
            .unwrap_or_else(|| panic!("a renderer"));
        let target = render
            .reference_frame("ref:1")
            .unwrap_or_else(|| panic!("a picture"));
        let image = (render.renderer.read_frame(target)).unwrap_or_else(|e| panic!("{e}"));
        (image.width, image.rgba)
    };
    let size = [width, (frame.len() / 4) as u32 / width];
    let args = json!({ "view": "ref:1", "size": size, "antialias": false });
    let shot = snapshot(&mut engine, args.clone());
    assert!(
        shot.0 >= 100 && height(&shot) >= 100,
        "a picture to compare"
    );
    assert!(shot.1 == frame, "the same bytes");
    // The picture compared shows both: clearing the mask changes it, and so does showing the
    // hidden faces again.
    run(
        &mut engine,
        "mask.set",
        json!({ "value": 0, "scope": "all" }),
    );
    let unmasked = snapshot(&mut engine, args.clone());
    assert!(unmasked.1 != shot.1, "the mask shows");
    run(&mut engine, "visibility.show_all", json!({}));
    assert!(
        snapshot(&mut engine, args).1 != unmasked.1,
        "hidden faces are not drawn"
    );
}

#[test]
fn a_contact_sheet_tiles_the_views_as_one_png() {
    let mut engine = headless();
    let sheet = |e: &mut Engine, views: Value, columns: Option<u32>| {
        let mut args = json!({ "views": views, "size": 300 });
        if let Some(columns) = columns {
            args["columns"] = json!(columns);
        }
        look(e, "view.contact_sheet", args)
    };
    let size = |shot: &Shot| (shot.0, height(shot));
    let (out, two) = sheet(&mut engine, json!(["front", "back"]), None);
    assert_eq!(size(&two), (300, 150), "two tiles side by side");
    assert_eq!(out["views"], json!(["front", "back"]), "in order");
    let wide = look(
        &mut engine,
        "view.contact_sheet",
        json!({ "views": ["front", "back"] }),
    )
    .1;
    assert_eq!(size(&wide), (1024, 512), "1024 px wide by default");
    let cases = [
        (json!(["front", "back", "top"]), None, (300, 300)),
        (json!(["front", "back", "top", "left"]), None, (300, 300)),
        (
            json!(["front", "back", "top", "left", "iso"]),
            None,
            (300, 200),
        ),
        (json!(["front", "back"]), Some(1), (300, 600)),
        (json!(["front", "back"]), Some(4), (300, 150)),
    ];
    for (views, columns, expected) in cases {
        let (_, shot) = sheet(&mut engine, views.clone(), columns);
        assert_eq!(size(&shot), expected, "{views} in {columns:?} columns");
    }
    // Each cell is the snapshot of its view, byte for byte below the label burned into its
    // top-left corner (18 pixels high).
    let (_, four) = sheet(&mut engine, json!(["front", "back", "top", "iso"]), None);
    for (i, view) in ["front", "back", "top", "iso"].into_iter().enumerate() {
        let alone = snapshot(&mut engine, json!({ "view": view, "size": 150 }));
        let (x0, y0) = (i as u32 % 2 * 150, i as u32 / 2 * 150);
        let cell: Vec<u8> = (18..150)
            .flat_map(|y| block(&four, x0, 150, y0 + y..y0 + y + 1))
            .collect();
        assert!(cell == block(&alone, 0, 150, 18..150), "{view}");
    }
}

#[test]
fn the_window_renders_looks_without_moving_its_camera() {
    let mut engine = windowed();
    scene(&mut engine);
    let view = |e: &mut Engine| run(e, "ui.describe", json!({}))["viewport"].clone();
    let before = view(&mut engine);
    let (out, shot) = look(
        &mut engine,
        "view.snapshot",
        json!({ "view": "top", "size": 200 }),
    );
    assert_eq!(
        (shot.0, out["views"].clone()),
        (200, json!(["top"])),
        "{out}"
    );
    let (_, shot) = look(
        &mut engine,
        "view.contact_sheet",
        json!({ "views": ["left", "right"], "size": 200 }),
    );
    assert_eq!((shot.0, height(&shot)), (200, 100), "a sheet too");
    let after = view(&mut engine);
    assert_eq!(
        after["camera"], before["camera"],
        "the window's camera stayed"
    );
}

#[test]
fn a_look_says_why_it_cannot_be_taken() {
    let refused =
        |engine: &mut Engine, command: &str, args: Value| match call(engine, command, args) {
            Err(e) => (json!(e.code), e.message, e.hint.unwrap_or_default()),
            Ok(out) => panic!("a look was taken: {out}"),
        };
    let refusal = |engine: &mut Engine| refused(engine, "view.snapshot", json!({}));
    let mut blind = Engine::new(EngineConfig::default()).unwrap_or_else(|e| panic!("{e}"));
    scene(&mut blind);
    let (code, message, hint) = refusal(&mut blind);
    assert_eq!(code, json!("E_PRECONDITION"), "{message}");
    assert!(message.contains("without a renderer"), "{message}");
    assert!(hint.contains("gigasculpt"), "{hint}");
    let mut empty = Engine::new(EngineConfig::default()).unwrap_or_else(|e| panic!("{e}"));
    empty.attach_renderer(Offscreen::host(None).unwrap_or_else(|e| panic!("{e}")));
    let (code, message, hint) = refusal(&mut empty);
    assert_eq!(code, json!("E_PRECONDITION"), "{message}");
    assert!(message.contains("no parts"), "{message}");
    assert!(hint.contains("primitive.sphere"), "how to make one: {hint}");
    let sheet = refused(
        &mut empty,
        "view.contact_sheet",
        json!({ "views": ["front"] }),
    );
    assert!(sheet.1.contains("no parts"), "a sheet too: {}", sheet.1);
    let unknown = Offscreen::host(Some("voodoo")).err().unwrap_or_default();
    assert!(unknown.contains("unknown adapter voodoo"), "{unknown}");
}

/// `material.list` names every shipped look with MatCap Gray selected; `material.select`
/// changes what `matcap` shading draws, headless and in the window, which reports it; an
/// unknown name is refused with the closest names.
#[test]
fn material_select_changes_what_matcap_shading_draws() {
    let mut engine = headless();
    let list = run(&mut engine, "material.list", json!({}));
    let names: Vec<&str> = (list["looks"].as_array().into_iter().flatten())
        .filter_map(|l| l["name"].as_str())
        .collect();
    assert!(
        names.len() >= 13 && names.contains(&"red-wax") && names.contains(&"chrome-a"),
        "every shipped look: {names:?}"
    );
    assert_eq!(
        (list["selected"].clone(), list["refused"].clone()),
        (json!("gray"), json!([])),
        "gray first, nothing refused"
    );
    let front = json!({ "view": "front", "size": 128 });
    let gray = snapshot(&mut engine, front.clone());
    let picked = run(&mut engine, "material.select", json!({ "name": "red-wax" }));
    assert_eq!(picked["selected"], json!("red-wax"), "{picked}");
    let wax = snapshot(&mut engine, front.clone());
    assert_ne!(gray, wax, "matcap shading draws the selected look");
    let clay = json!({ "view": "front", "size": 128, "shading": "clay" });
    let clay_wax = snapshot(&mut engine, clay.clone());
    run(&mut engine, "material.select", json!({ "name": "gray" }));
    assert_eq!(
        snapshot(&mut engine, front),
        gray,
        "back to gray, byte for byte"
    );
    assert_eq!(
        snapshot(&mut engine, clay),
        clay_wax,
        "clay ignores the choice"
    );
    let refused = call(&mut engine, "material.select", json!({ "name": "red wax" }));
    let e = refused
        .err()
        .unwrap_or_else(|| panic!("an unknown look is refused"));
    assert_eq!(json!(e.code), json!("E_NOT_FOUND"), "{}", e.message);
    assert!(
        e.suggestions.contains(&"red-wax".to_string()),
        "{:?}",
        e.suggestions
    );
    let hint = e.hint.unwrap_or_default();
    assert!(
        hint.contains("material.list"),
        "how to find the names: {hint}"
    );

    let mut window = windowed();
    scene(&mut window);
    let shading = |e: &mut Engine| run(e, "ui.describe", json!({}))["viewport"]["shading"].clone();
    assert_eq!(shading(&mut window), json!("matcap gray"));
    let before = snapshot(&mut window, json!({ "view": "front", "size": 128 }));
    run(&mut window, "material.select", json!({ "name": "skin-4" }));
    assert_eq!(
        shading(&mut window),
        json!("matcap skin-4"),
        "the window says so"
    );
    let after = snapshot(&mut window, json!({ "view": "front", "size": 128 }));
    assert_ne!(before, after, "the window's looks draw it");
}

/// Looks are anti-aliased: along the slanted edges of a flat quad, whose surface is one color
/// in `normals` shading, pixels blend the surface with the background instead of being one or
/// the other. With `antialias` false every pixel is one or the other.
#[test]
fn looks_smooth_their_edges() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap_or_else(|e| panic!("{e}"));
    engine.attach_renderer(Offscreen::host(Some("lavapipe")).unwrap_or_else(|e| panic!("{e}")));
    let quad = json!({
        "positions": [[-1.0, -0.4, 0.0], [0.4, -1.0, 0.0], [1.0, 0.4, 0.0], [-0.4, 1.0, 0.0]],
        "faces": [[0, 1, 2, 3]],
    });
    run(
        &mut engine,
        "part.create",
        json!({ "name": "quad", "mesh": quad }),
    );
    // `antialias` left out (the default), or given.
    let blended = |engine: &mut Engine, antialias: Option<bool>| {
        let mut args = json!({ "view": "front", "size": 64, "shading": "normals" });
        if let Some(on) = antialias {
            args["antialias"] = json!(on);
        }
        let shot = snapshot(engine, args);
        let middle = ((32 * shot.0 + 32) * 4) as usize;
        let surface = shot.1[middle..middle + 4].to_vec();
        (shot.1.chunks(4))
            .filter(|p| *p != BACKGROUND && *p != surface.as_slice())
            .count()
    };
    let smooth = blended(&mut engine, None);
    assert!(
        smooth > 4 * 16,
        "the quad's four edges blend into the background: {smooth} pixels"
    );
    assert_eq!(
        blended(&mut engine, Some(false)),
        0,
        "one sample: no pixel blends"
    );
}
