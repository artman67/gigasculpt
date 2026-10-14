//! `image.diff`: two PNG files of the same size compared pixel by pixel, as numbers and as an
//! optional difference image, and files of different sizes refused.
//!
//! The query takes `a` and `b` (paths to PNG files) and an optional `path` for the difference
//! image. Its result holds `differing` (how many pixels differ in any channel), `max` (the
//! largest difference in one channel, in levels), `mean` (the mean absolute difference per
//! channel over every pixel, as `[r, g, b, a]`) and `box` (the differing pixels' bounds as
//! `{ "min": [x, y], "max": [x, y] }`, both corners inclusive, row 0 at the top; null when
//! nothing differs).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use gs_render::Image;
use gs_testkit::{code, success};
use serde_json::{Value, json};

use crate::support::agent;

/// 7 wide and 5 high, so a swapped axis shows; no pixel is uniform with its neighbours.
const W: u32 = 7;
const H: u32 = 5;

/// A gradient with bright pixels in it, so the difference image cannot be the input itself.
fn picture(width: u32, height: u32) -> Image {
    let mut image = Image::filled(width, height, [0, 0, 0, 255]);
    for y in 0..height {
        for x in 0..width {
            let i = ((y * width + x) * 4) as usize;
            let v = (x * 31 + y * 17) as u8;
            image.rgba[i..i + 3].copy_from_slice(&[200u8.wrapping_add(v), v, 255 - v]);
        }
    }
    image
}

/// `image` with channel `c` of pixel (`x`, `y`) moved by `by` levels.
fn changed(image: &Image, x: u32, y: u32, c: usize, by: i16) -> Image {
    let mut out = image.clone();
    let i = ((y * image.width + x) * 4) as usize + c;
    out.rgba[i] =
        u8::try_from(i16::from(out.rgba[i]) + by).unwrap_or_else(|e| panic!("leaves 0..=255: {e}"));
    out
}

/// A fresh path under the target folder.
fn path(name: &str) -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("image-diff-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("scratch folder: {e}"));
    dir.join(name).to_string_lossy().into_owned()
}

/// Writes `image` as a PNG file; its path.
fn save(image: &Image, name: &str) -> String {
    let file = path(name);
    std::fs::write(&file, image.png().unwrap_or_else(|e| panic!("encode: {e}")))
        .unwrap_or_else(|e| panic!("{file}: {e}"));
    file
}

fn assert_mean(result: &Value, want: [f64; 4]) {
    let got: Vec<f64> = (result["mean"].as_array())
        .unwrap_or_else(|| panic!("no mean list: {result}"))
        .iter()
        .filter_map(Value::as_f64)
        .collect();
    assert_eq!(got.len(), 4, "one mean per channel, r g b a: {result}");
    for (c, (g, w)) in got.iter().zip(want).enumerate() {
        assert!(
            (g - w).abs() <= 1e-9,
            "mean of channel {c}: got {g}, want {w}"
        );
    }
}

/// The pixel box `image.diff` should report.
fn pixel_box(min: [u32; 2], max: [u32; 2]) -> Value {
    json!({ "min": min, "max": max })
}

#[test]
fn an_image_against_itself_differs_nowhere() {
    let mut s = agent();
    let a = save(&picture(W, H), "a.png");
    let copy = save(&picture(W, H), "copy.png");
    for b in [&a, &copy] {
        let out = s.query("image.diff", json!({ "a": a, "b": b }));
        assert_eq!(out["differing"], json!(0), "no pixel differs: {out}");
        assert_eq!(out["max"], json!(0), "{out}");
        assert_mean(&out, [0.0; 4]);
        assert!(out["box"].is_null(), "an empty box is null: {out}");
    }
}

#[test]
fn one_pixel_changed_by_10_levels_is_counted_measured_and_boxed() {
    let mut s = agent();
    let base = picture(W, H);
    // Green of the pixel in column 5, row 2, made darker, so a wrapping subtraction shows
    // in one of the two directions.
    let a = save(&base, "a.png");
    let b = save(&changed(&base, 5, 2, 1, -10), "b.png");
    for (first, second) in [(&a, &b), (&b, &a)] {
        let out = s.query("image.diff", json!({ "a": first, "b": second }));
        assert_eq!(out["differing"], json!(1), "one pixel: {out}");
        assert_eq!(out["max"], json!(10), "10 levels: {out}");
        assert_mean(&out, [0.0, 10.0 / f64::from(W * H), 0.0, 0.0]);
        assert_eq!(out["box"], pixel_box([5, 2], [5, 2]), "x then y: {out}");
    }
}

#[test]
fn the_box_spans_every_differing_pixel_and_alpha_counts() {
    let mut s = agent();
    let base = picture(W, H);
    let one = changed(&base, 1, 3, 0, 10);
    let both = changed(&one, 4, 0, 3, -200);
    let out = s.query(
        "image.diff",
        json!({ "a": save(&base, "a.png"), "b": save(&both, "b.png") }),
    );
    assert_eq!(out["differing"], json!(2), "{out}");
    assert_eq!(out["max"], json!(200), "the largest one, in alpha: {out}");
    let n = f64::from(W * H);
    assert_mean(&out, [10.0 / n, 0.0, 0.0, 200.0 / n]);
    assert_eq!(out["box"], pixel_box([1, 0], [4, 3]), "{out}");
}

/// Width, height and how bright each pixel is (the sum of its color channels).
fn brightness(file: &str) -> (u32, u32, Vec<u32>) {
    let bytes = std::fs::read(file).unwrap_or_else(|e| panic!("{file}: {e}"));
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder
        .read_info()
        .unwrap_or_else(|e| panic!("{file}: {e}"));
    let mut data = vec![0; reader.output_buffer_size().unwrap_or_default()];
    let info = reader
        .next_frame(&mut data)
        .unwrap_or_else(|e| panic!("{file}: {e}"));
    data.truncate(info.buffer_size());
    let (channels, colors) = match info.color_type {
        png::ColorType::Grayscale => (1, 1),
        png::ColorType::GrayscaleAlpha => (2, 1),
        png::ColorType::Rgb => (3, 3),
        png::ColorType::Rgba => (4, 3),
        png::ColorType::Indexed => panic!("expanded by the decoder"),
    };
    let sums = (data.chunks(channels))
        .map(|p| {
            p[..colors]
                .iter()
                .map(|&v| u32::from(v) * 3 / colors as u32)
                .sum()
        })
        .collect();
    (info.width, info.height, sums)
}

#[test]
fn the_difference_image_shows_the_changed_pixel_brightest() {
    let mut s = agent();
    let base = picture(W, H);
    let out_path = path("diff.png");
    let args = json!({
        "a": save(&base, "a.png"),
        "b": save(&changed(&base, 5, 2, 2, 10), "b.png"),
        "path": out_path,
    });
    let out = s.query("image.diff", args);
    assert_eq!(out["differing"], json!(1), "{out}");
    let (width, height, sums) = brightness(&out_path);
    assert_eq!((width, height), (W, H), "the inputs' size");
    let spot = (2 * W + 5) as usize;
    for (i, &sum) in sums.iter().enumerate() {
        if i != spot {
            assert!(
                sum < sums[spot],
                "pixel ({}, {}) is as bright as the changed one at (5, 2): {sum} >= {}",
                i as u32 % W,
                i as u32 / W,
                sums[spot]
            );
        }
    }
}

#[test]
fn images_of_different_sizes_are_refused_naming_both() {
    let mut s = agent();
    let wide = save(&picture(W, H), "wide.png");
    let narrow = save(&picture(W - 1, H), "narrow.png");
    let out_path = path("never.png");
    let response = s.call(
        "image.diff",
        json!({ "a": wide, "b": narrow, "path": out_path }),
    );
    let err = gs_testkit::failure(response, "image.diff of 7x5 against 6x5");
    assert_eq!(code(err.code), "E_ARGS_INVALID", "{}", err.message);
    for size in ["7x5", "6x5"] {
        assert!(
            err.message.contains(size),
            "the message names {size} (width x height): {}",
            err.message
        );
    }
    assert!(
        !std::path::Path::new(&out_path).exists(),
        "no difference image is written"
    );
    // Identical sizes still work afterwards, so the refusal came from the sizes.
    success(
        s.call("image.diff", json!({ "a": wide, "b": wide })),
        "same size",
    );
}
