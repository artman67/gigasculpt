//! The glue: platform input to egui events, the mouse's held buttons to pen samples, pen
//! samples to pointer events, egui's output to the renderer's interface layer.

use egui::{Event, Key, PointerButton, Pos2, TextureId};
use gs_platform::{Button, Input, Modifiers as Held};
use gs_protocol::PenSample;
use gs_render::UiTexture;
use gs_ui::{MousePen, PenRouter, events, fonts, key, layer, split_pen};

#[test]
fn sdl_key_names_become_egui_keys() {
    for (sdl, egui) in [
        ("Return", Key::Enter),
        ("Keypad Enter", Key::Enter),
        ("Left", Key::ArrowLeft),
        ("Right", Key::ArrowRight),
        ("Up", Key::ArrowUp),
        ("Down", Key::ArrowDown),
        ("Space", Key::Space),
        ("Z", Key::Z),
        ("Escape", Key::Escape),
        ("F1", Key::F1),
    ] {
        assert_eq!(key(sdl), Some(egui), "{sdl}");
    }
    assert_eq!(key("Left Ctrl"), None, "modifier keys are not egui keys");
}

#[test]
fn platform_input_becomes_egui_events_in_points() {
    let ctrl = Held {
        ctrl: true,
        ..Held::default()
    };
    let held = egui::Modifiers::CTRL;
    let inputs = [
        Input::MouseMove { x: 20.0, y: 10.0 },
        Input::MouseButton {
            button: Button::Right,
            down: true,
            x: 20.0,
            y: 10.0,
        },
        Input::MouseButton {
            button: Button::Middle,
            down: false,
            x: 2.0,
            y: 4.0,
        },
        Input::MouseButton {
            button: Button::Left,
            down: true,
            x: 2.0,
            y: 4.0,
        },
        Input::Wheel { dx: 0.0, dy: -1.0 },
        Input::Key {
            key: "Z".into(),
            down: true,
            repeat: false,
            modifiers: ctrl,
        },
        Input::Key {
            key: "Left Shift".into(),
            down: true,
            repeat: false,
            modifiers: ctrl,
        },
        Input::Text("x".into()),
        Input::PointerGone,
        Input::Resized,
        Input::Close,
        Input::Pen(PenSample::hover(0, 1.0, 1.0)),
    ];
    let got = events(&inputs, 2.0, held);
    let button = |pos: Pos2, button, pressed| Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: held,
    };
    let expected = vec![
        Event::PointerMoved(Pos2::new(10.0, 5.0)),
        button(Pos2::new(10.0, 5.0), PointerButton::Secondary, true),
        button(Pos2::new(1.0, 2.0), PointerButton::Middle, false),
        button(Pos2::new(1.0, 2.0), PointerButton::Primary, true),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: held,
        },
        Event::Key {
            key: Key::Z,
            physical_key: None,
            pressed: true,
            repeat: false,
            // egui's command is Cmd on macOS, Ctrl elsewhere.
            modifiers: egui::Modifiers {
                ctrl: true,
                command: !cfg!(target_os = "macos"),
                ..egui::Modifiers::NONE
            },
        },
        Event::Text("x".into()),
        Event::PointerGone,
    ];
    assert_eq!(got, expected);
}

#[test]
fn pen_tip_is_the_primary_button_and_the_barrel_the_secondary() {
    let mut router = PenRouter::default();
    let held = egui::Modifiers::NONE;
    let at = |x, y| PenSample::hover(0, x, y);
    let hover = at(10.0, 20.0);
    let touch = PenSample {
        contact: true,
        pressure: 0.5,
        ..at(12.0, 20.0)
    };
    let barrel = PenSample {
        buttons: 1,
        ..at(14.0, 20.0)
    };
    let gone = PenSample {
        in_proximity: false,
        ..at(14.0, 20.0)
    };
    let pos = |x| Pos2::new(x, 10.0);
    let press = |x, button, pressed| Event::PointerButton {
        pos: pos(x),
        button,
        pressed,
        modifiers: held,
    };
    assert_eq!(
        router.route(&hover, 2.0, held),
        vec![Event::PointerMoved(pos(5.0))]
    );
    assert_eq!(
        router.route(&touch, 2.0, held),
        vec![
            Event::PointerMoved(pos(6.0)),
            press(6.0, PointerButton::Primary, true)
        ]
    );
    assert_eq!(
        router.route(&barrel, 2.0, held),
        vec![
            Event::PointerMoved(pos(7.0)),
            press(7.0, PointerButton::Primary, false),
            press(7.0, PointerButton::Secondary, true),
        ],
        "lifting while the barrel button goes down"
    );
    assert_eq!(
        router.route(&gone, 2.0, held),
        vec![
            Event::PointerMoved(pos(7.0)),
            press(7.0, PointerButton::Secondary, false),
            Event::PointerGone,
        ],
        "leaving range lets go of everything"
    );
    let back = PenSample {
        contact: true,
        ..at(14.0, 20.0)
    };
    assert_eq!(
        router.route(&back, 2.0, held),
        vec![
            Event::PointerMoved(pos(7.0)),
            press(7.0, PointerButton::Primary, true)
        ],
        "a new session starts from nothing held"
    );
}

/// One egui frame drawing a rectangle at (10, 10)-(20, 20) points and a user image.
fn frame(ctx: &egui::Context, keep: Option<&egui::TextureHandle>) -> egui::FullOutput {
    let mut raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            Pos2::ZERO,
            egui::vec2(100.0, 80.0),
        )),
        ..egui::RawInput::default()
    };
    raw.viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .native_pixels_per_point = Some(2.0);
    ctx.run_ui(raw, |ui| {
        ui.set_clip_rect(egui::Rect::from_min_max(
            Pos2::new(5.0, 5.0),
            Pos2::new(60.0, 40.0),
        ));
        let painter = ui.painter();
        let rect = egui::Rect::from_min_max(Pos2::new(10.0, 10.0), Pos2::new(20.0, 20.0));
        painter.rect_filled(rect, 0.0, egui::Color32::RED);
        let uv = egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        let image = rect.translate(egui::vec2(30.0, 0.0));
        painter.image(TextureId::User(3), image, uv, egui::Color32::WHITE);
        if let Some(handle) = keep {
            painter.image(handle.id(), image, uv, egui::Color32::WHITE);
        }
    })
}

#[test]
fn egui_output_becomes_an_interface_layer_in_pixels() {
    let ctx = egui::Context::default();
    ctx.set_fonts(fonts());
    let mut freed = Vec::new();
    let first = layer(&ctx, frame(&ctx, None), &mut freed);
    let font_atlas = first.textures.iter().any(|t| {
        matches!(t, UiTexture::Set { id: 0, at: None, image } if image.width > 0 && image.rgba.len() == (image.width * image.height * 4) as usize)
    });
    assert!(font_atlas, "the font atlas arrives whole first");
    let red = first
        .meshes
        .iter()
        .flat_map(|m| &m.vertices)
        .filter(|v| v.color == [255, 0, 0, 255])
        .map(|v| v.pos)
        .collect::<Vec<_>>();
    let (lo, hi) = red.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
        (lo.min(p[0]).min(p[1]), hi.max(p[0]).max(p[1]))
    });
    assert!(
        (19.0..=21.0).contains(&lo) && (39.0..=41.0).contains(&hi),
        "points times 2, give or take the half-pixel feather: {lo}..{hi}"
    );
    let clips: Vec<[f32; 4]> = first
        .meshes
        .iter()
        .filter(|m| m.vertices.iter().any(|v| v.color == [255, 0, 0, 255]))
        .map(|m| m.clip)
        .collect();
    assert_eq!(
        clips,
        vec![[10.0, 10.0, 120.0, 80.0]],
        "the clip rectangle, in pixels"
    );
    let user = first.meshes.iter().find(|m| m.texture == 7);
    assert!(
        user.is_some_and(|m| m.clip[2] > 0.0),
        "user texture 3 is id 7"
    );

    let image = egui::ColorImage::filled([2, 2], egui::Color32::BLUE);
    let handle = ctx.load_texture("swatch", image, egui::TextureOptions::LINEAR);
    let egui::TextureId::Managed(n) = handle.id() else {
        panic!("a managed texture")
    };
    let second = layer(&ctx, frame(&ctx, Some(&handle)), &mut freed);
    let swatch = second
        .textures
        .iter()
        .any(|t| matches!(t, UiTexture::Set { id, image, .. } if *id == n * 2 && image.smooth));
    assert!(
        swatch,
        "a loaded texture arrives, smooth: {:?}",
        second.textures.len()
    );
    assert!(
        second.meshes.iter().any(|m| m.texture == n * 2),
        "and is drawn"
    );
    drop(handle);
    let third = layer(&ctx, frame(&ctx, None), &mut freed);
    assert!(
        !third.textures.contains(&UiTexture::Free(n * 2)),
        "not freed while the frame that drops it draws"
    );
    let fourth = layer(&ctx, frame(&ctx, None), &mut freed);
    assert_eq!(
        fourth.textures.first(),
        Some(&UiTexture::Free(n * 2)),
        "freed first thing in the next frame"
    );
}

#[test]
fn a_pump_splits_into_pen_samples_and_the_rest_in_order() {
    let pen = |x| Input::Pen(PenSample::hover(0, x, 0.0));
    let inputs = [
        pen(1.0),
        Input::Resized,
        pen(2.0),
        Input::Text("a".into()),
        pen(3.0),
    ];
    let (samples, others) = split_pen(&inputs);
    let xs: Vec<f64> = samples.iter().map(|s| s.x).collect();
    assert_eq!(xs, [1.0, 2.0, 3.0], "samples in order");
    assert_eq!(
        others,
        vec![Input::Resized, Input::Text("a".into())],
        "the rest in order"
    );
}

#[test]
fn the_mouse_is_a_pen_hovering_and_while_a_button_is_held() {
    let mut mouse = MousePen::default();
    let button = |button, down| Input::MouseButton {
        button,
        down,
        x: 5.0,
        y: 6.0,
    };
    let out = mouse.convert(
        &[
            Input::MouseMove { x: 1.0, y: 2.0 },
            button(Button::Left, true),
            Input::MouseMove { x: 7.0, y: 8.0 },
            button(Button::Right, true),
            button(Button::Left, false),
            Input::Wheel { dx: 0.0, dy: 1.0 },
            button(Button::Right, false),
            button(Button::Middle, true),
            button(Button::Middle, false),
            Input::PointerGone,
        ],
        100,
    );
    let pen = |i: &Input| match i {
        Input::Pen(s) => Some((s.t_ns, s.contact, s.pressure, s.buttons, s.in_proximity)),
        _ => None,
    };
    let samples: Vec<_> = out.iter().filter_map(pen).collect();
    assert_eq!(
        samples,
        [
            (100, false, 0.0, 0, true),
            (101, true, 1.0, 0, true),
            (102, true, 1.0, 0, true),
            (103, true, 1.0, 1, true),
            (104, false, 0.0, 1, true),
            (105, false, 0.0, 0, true),
            (106, false, 0.0, 2, true),
            (107, false, 0.0, 0, true),
            (108, false, 0.0, 0, false),
        ],
        "motion with nothing held hovers at pressure 0; left is the tip at pressure 1, right \
         barrel 1, middle barrel 2; a release hovers; leaving the window leaves range; times \
         keep their order"
    );
    let others: Vec<&Input> = out.iter().filter(|i| pen(i).is_none()).collect();
    assert_eq!(
        others,
        [&Input::Wheel { dx: 0.0, dy: 1.0 }, &Input::PointerGone],
        "the wheel stays the mouse's, and the pointer leaves after the pen"
    );
    let tilt = out.iter().find_map(|i| match i {
        Input::Pen(s) => Some((s.tilt_x, s.tilt_y)),
        _ => None,
    });
    assert_eq!(tilt, Some((0.0, 0.0)), "no tilt");
}
