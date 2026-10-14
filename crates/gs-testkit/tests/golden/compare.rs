//! The comparison rule's own negative tests (ARCHITECTURE 10.1 item 4): a 0.5% weaker dent on
//! a zebra scene fails, GPU noise passes, a one-pixel change fails byte equality, and each
//! limit of the look rule decides where it should.

use crate::common::{dented_picture, hardware, lavapipe};
use gs_render::Image;
use gs_testkit::golden::{
    LOOK_LEVELS, LOOK_MAX, LOOK_PIXEL, LOOK_PIXELS, Look, Rule, ZEBRA_MAX, compare, diff_map,
};

#[test]
fn a_weaker_dent_fails_and_gpu_noise_passes() {
    let r = lavapipe();
    let base = dented_picture(r, 0.12, &Look::Zebra);
    let weak = dented_picture(r, 0.1194, &Look::Zebra);
    let across = compare(&base, &weak, false, true);
    assert!(
        !across.pass && across.rule == Rule::Zebra && across.flip_max > ZEBRA_MAX,
        "a 0.5% weaker stroke fails the zebra rule (spike: FLIP 0.67): {}",
        across.summary()
    );
    let same = compare(&base, &weak, true, true);
    assert!(
        !same.pass && same.rule == Rule::Bytes && same.differing > 1000,
        "and byte equality: {}",
        same.summary()
    );
    let clay = dented_picture(r, 0.12, &Look::Clay);
    for (kind, gpu) in hardware() {
        let noise = compare(&base, &dented_picture(gpu, 0.12, &Look::Zebra), false, true);
        assert!(
            noise.pass && noise.flip_max > 0.0,
            "{kind:?} renders the same stroke within the zebra rule: {}",
            noise.summary()
        );
        let look = compare(&clay, &dented_picture(gpu, 0.12, &Look::Clay), false, false);
        assert!(
            look.pass,
            "{kind:?} renders clay within the look rule: {}",
            look.summary()
        );
        assert!(
            !compare(&base, &dented_picture(gpu, 0.12, &Look::Zebra), true, true).pass,
            "{kind:?} is not byte-identical, so the renderer key matters"
        );
    }
}

/// `image` with `count` pixels of a 64-pixel-wide stripe pattern in the middle raised by
/// `levels` in every color channel.
fn bumped(image: &Image, count: u32, levels: u8) -> Image {
    let mut out = image.clone();
    for k in 0..count {
        let (x, y) = (192 + (k % 64) * 2, 200 + (k / 64) * 2);
        let i = ((y * image.width + x) * 4) as usize;
        for c in &mut out.rgba[i..i + 3] {
            *c = c.saturating_add(levels);
        }
    }
    out
}

#[test]
fn one_pixel_fails_byte_equality_only() {
    let base = dented_picture(lavapipe(), 0.12, &Look::Clay);
    let one = bumped(&base, 1, 1);
    let same = compare(&base, &one, true, false);
    assert!(
        !same.pass && same.differing == 1 && same.max_level == 1,
        "one pixel one level off fails byte equality: {}",
        same.summary()
    );
    for zebra in [false, true] {
        let across = compare(&base, &one, false, zebra);
        assert!(
            across.pass,
            "but passes across renderers: {}",
            across.summary()
        );
    }
    let equal = compare(&base, &base.clone(), true, false);
    assert!(
        equal.pass && equal.differing == 0 && equal.flip_max == 0.0,
        "identical images pass without FLIP"
    );
}

#[test]
fn look_rule_limits_decide() {
    let base = dented_picture(lavapipe(), 0.12, &Look::Clay);
    let check = |img: &Image| compare(&base, img, false, false);
    // Within 2 levels everywhere: passes however many pixels moved, whatever FLIP says.
    let mut shifted = base.clone();
    shifted
        .rgba
        .iter_mut()
        .for_each(|c| *c = c.saturating_add(LOOK_LEVELS));
    let v = check(&shifted);
    assert!(
        v.pass && v.max_level == LOOK_LEVELS,
        "a whole-image 2-level shift passes: {}",
        v.summary()
    );
    // FLIP spreads each changed pixel over its neighbours, so the counts are of pixels FLIP
    // flags, not of pixels changed (measured: 2 pixels 60 levels off flag 23).
    // Two pixels far off: few flagged, but past FLIP 0.15, so it fails on the worst pixel.
    let v = check(&bumped(&base, 2, 60));
    assert!(
        !v.pass && v.flip_over <= LOOK_PIXELS && v.flip_max > LOOK_MAX,
        "2 pixels far off fail on the worst pixel: {}",
        v.summary()
    );
    // Forty pixels moderately off: none past 0.15, but over 50 flagged past 0.06.
    let v = check(&bumped(&base, 40, 15));
    assert!(
        !v.pass && v.flip_over > LOOK_PIXELS && v.flip_max <= LOOK_MAX,
        "40 pixels 15 levels off fail on the count: {}",
        v.summary()
    );
    // Ten pixels moderately off: past 2 levels, but within both FLIP limits, so it passes.
    let v = check(&bumped(&base, 10, 15));
    assert!(
        v.pass
            && v.max_level > LOOK_LEVELS
            && v.flip_over <= LOOK_PIXELS
            && v.flip_max > LOOK_PIXEL,
        "10 pixels 15 levels off pass on FLIP: {}",
        v.summary()
    );
    // Never the mean: the whole-image shift has the higher FLIP mean, yet it passes and the
    // two far-off pixels fail.
    let mean = |img: &Image| {
        let rgb = |i: &Image| {
            let data: Vec<u8> = i.rgba.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
            nv_flip::FlipImageRgb8::with_data(i.width, i.height, &data)
        };
        let errors = nv_flip::flip(rgb(&base), rgb(img), nv_flip::DEFAULT_PIXELS_PER_DEGREE);
        let errors = errors.to_vec();
        errors.iter().map(|&e| f64::from(e)).sum::<f64>() / errors.len() as f64
    };
    let spot = bumped(&base, 2, 60);
    assert!(
        mean(&shifted) > 10.0 * mean(&spot) && check(&shifted).pass && !check(&spot).pass,
        "the FLIP mean would rank these the wrong way round ({} against {})",
        mean(&shifted),
        mean(&spot)
    );
    let small = Image::filled(8, 8, [0, 0, 0, 255]);
    for v in [check(&small), compare(&small, &base, false, false)] {
        assert!(
            !v.pass && v.differing == base.rgba.len() / 4,
            "a picture of another size fails, every pixel of the larger counted"
        );
    }
}

#[test]
fn diff_map_shows_where_images_differ() {
    let base = dented_picture(lavapipe(), 0.12, &Look::Clay);
    let calm = diff_map(&base, &base);
    let changed = diff_map(&base, &bumped(&base, 64, 120));
    assert_eq!(
        (changed.width, changed.height),
        (base.width, base.height),
        "the map is the image's size"
    );
    let luma = |i: &Image, x: u32, y: u32| {
        let p = i.pixel(x, y);
        u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])
    };
    assert!(
        calm.rgba.chunks(4).all(|p| p == calm.pixel(0, 0)),
        "identical images map to one color"
    );
    assert!(
        luma(&changed, 192, 200) > luma(&calm, 192, 200) + 200,
        "a changed pixel lights up"
    );
    assert_eq!(
        luma(&changed, 10, 10),
        luma(&calm, 10, 10),
        "far from the change, nothing"
    );
    let other = diff_map(&base, &Image::filled(4, 2, [0; 4]));
    assert_eq!(
        other,
        Image::filled(4, 2, [255, 0, 0, 255]),
        "different sizes map to solid red"
    );
}
