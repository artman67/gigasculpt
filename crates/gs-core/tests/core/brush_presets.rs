//! Preset files with mistakes are refused with a message naming the key, and every edge value
//! the format allows is accepted (ARCHITECTURE 10.1, kind 1: nonsense input gives an error and
//! no result).

use gs_core::BrushPreset;

use crate::common::ok;

/// A preset with `kernel` as its kernel table and `rest` appended.
fn with(kernel: &str, rest: &str) -> String {
    format!(
        "name = \"T\"\nid = \"t\"\nfamily = \"Build\"\nstrength = 0.5\n[kernel]\n{kernel}\n{rest}\n"
    )
}

const SMOOTH: &str = "type = \"smooth\"\nmode = \"uniform\"\ngain = 1";
const MOVE: &str = "type = \"move\"\nselect = \"ball\"\n";
const DRAG: &str = "[stroke]\nclass = \"drag\"";

/// Whole files with a mistake outside the kernel, and what the message must say.
const BAD_FILES: &[(&str, &str)] = &[
    ("name = \"T\"\nstrength = 0.5\n", "does not parse"),
    (
        "name = \" \"\nid = \"t\"\nfamily = \"Build\"\nstrength = 0.5\n[kernel]\ntype = \"hook\"",
        "name must not be empty",
    ),
    (
        "name = \"T\"\nid = \"t\"\nfamily = \"Build\"\nstrength = 1.5\n[kernel]\ntype = \"hook\"",
        "strength must be from 0 to 1",
    ),
    (
        "name = \"T\"\nid = \"t\"\nfamily = \"Build\"\nstrength = nan\n[kernel]\ntype = \"hook\"",
        "strength must be",
    ),
    (
        "name = \"T\"\nid = \"t\"\nstrength = 0.5\n[kernel]\ntype = \"hook\"",
        "missing field `family`",
    ),
    (
        "name = \"T\"\nid = \"t\"\nfamily = \"Biuld\"\nstrength = 0.5\n[kernel]\ntype = \"hook\"",
        "unknown variant `Biuld`",
    ),
    (
        "name = \"T\"\nid = \"t\"\nfamily = \"Paint\"\nstrength = 0.5\nmode = \"off\"\n[kernel]\n\
         type = \"hook\"",
        "mode off",
    ),
];

/// (kernel table, other tables, what the message must say).
const BAD: &[(&str, &str, &str)] = &[
    (
        "type = \"offset\"\nterms = []",
        "",
        "kernel.terms needs 1 or 2 terms, got 0",
    ),
    (
        "type = \"offset\"\nterms = [{dir = \"x\", gain = 1}, {dir = \"y\", gain = 1}, {dir = \"z\", gain = 1}]",
        "",
        "got 3",
    ),
    (
        "type = \"offset\"\nterms = [{dir = \"x\", gain = 4.5}]",
        "",
        "kernel.terms[0].gain",
    ),
    (
        "type = \"offset\"\nterms = [{dir = \"x\", gain = 1}, {dir = \"x\", gain = 1, exponent = 0.1}]",
        "",
        "kernel.terms[1].exponent",
    ),
    (
        "type = \"offset\"\nterms = [{dir = \"x\", gain = 1, tangent = true}]",
        "",
        "tangent only applies to to_center",
    ),
    (
        "type = \"plane\"\noffset = 0.1\ngain = 1",
        "",
        "unknown field `offset`",
    ),
    (
        "type = \"plane\"\ngain = 1",
        "[depth]\nimbed = 2",
        "depth.imbed",
    ),
    (SMOOTH, "[depth]\nimbed = 0.1", "depth.imbed only moves"),
    (
        "type = \"plane\"\ngain = 1",
        "[depth]\nimbed = 0.1\n[stroke]\nclass = \"gesture\"\nshape = \"curve\"",
        "depth.imbed only moves",
    ),
    (
        SMOOTH,
        "[tip.alpha]\nimage = \" \"",
        "tip.alpha.image must name",
    ),
    (
        SMOOTH,
        "[tip.alpha]\nimage = \"blake3:abc\"",
        "64 lowercase hex",
    ),
    (
        SMOOTH,
        "[tip.alpha]\nimage = \"a\"\nmid = 1.5",
        "tip.alpha.mid",
    ),
    (
        SMOOTH,
        "[tip.alpha]\nimage = \"a\"\nturn = 4",
        "tip.alpha.turn",
    ),
    (
        SMOOTH,
        "[tip.alpha]\nimage = \"a\"\nblur = 65",
        "tip.alpha.blur",
    ),
    (
        SMOOTH,
        "[tip.alpha]\nimage = \"a\"\nnoise = 2",
        "tip.alpha.noise",
    ),
    (
        SMOOTH,
        "[tip.alpha]\nimage = \"a\"\nnoise_scale = 0",
        "tip.alpha.noise_scale",
    ),
    (
        SMOOTH,
        "[tip.alpha]\nimage = \"a\"\ncontrast = 17",
        "tip.alpha.contrast",
    ),
    (
        SMOOTH,
        "[tip.alpha]\nimage = \"a\"\nradial_fade = 9",
        "tip.alpha.radial_fade",
    ),
    (
        SMOOTH,
        "[tip.alpha]\nimage = \"a\"\ntiles = [0, 1]",
        "tip.alpha.tiles",
    ),
    (
        SMOOTH,
        "[tip.alpha]\nimage = \"a\"\nflip = \"sideways\"",
        "unknown variant `sideways`",
    ),
    (SMOOTH, "[tip.alpha]\nmid = 0.5", "missing field `image`"),
    (
        SMOOTH,
        "[tip]\nalpha = \"grain\"",
        "expected struct AlphaSpec",
    ),
    (
        SMOOTH,
        "[pressure]\nsize = { points = [[0, 1], [1, 0]], smooth = true, sharp = [2] }",
        "sharp point 2 does not exist",
    ),
    (
        SMOOTH,
        "[pressure]\nmodifier = { points = [[0, 1], [0.5, 2], [1, 0]], smooth = true, bend = 1 }",
        "brush preset does not parse",
    ),
    (
        SMOOTH,
        "[post]\nauto_smooth_mode = \"wavy\"",
        "unknown variant",
    ),
    ("type = \"plane\"\ngain = -1", "", "kernel.gain"),
    (
        "type = \"plane\"\ngain = 1\nheight = -1",
        "",
        "kernel.height",
    ),
    ("type = \"plane\"\ngain = 1\ndepth = 17", "", "kernel.depth"),
    (
        "type = \"smooth\"\nmode = \"uniform\"\ngain = 101",
        "",
        "kernel.gain",
    ),
    (
        "type = \"move\"\nselect = \"ball\"\nsupport = 0.5",
        DRAG,
        "kernel.support",
    ),
    (
        "type = \"move\"\nselect = \"ball\"\npoisson = 0.6",
        DRAG,
        "kernel.poisson",
    ),
    ("type = \"hook\"\npinch = 2", "", "kernel.pinch"),
    (
        "type = \"attribute\"\ncolumn = \"mask\"",
        "",
        "does not parse",
    ),
    (
        "type = \"attribute\"\ncolumn = \"mask\"\ngain = 200",
        "",
        "kernel.gain",
    ),
    (
        "type = \"attribute\"\ngain = 1\ncolumn = \"group\"\nop = \"blur\"",
        "",
        "blur and sharpen only apply to the mask",
    ),
    (
        "type = \"attribute\"\ngain = 1\ncolumn = \"color\"\nvalue = 4294967295",
        "",
        "a brush paints color with a [paint] table",
    ),
    (
        "type = \"attribute\"\ngain = 1\ncolumn = \"mask\"\nvalue = 1",
        "[paint]\nchannel = \"color\"",
        "[paint] goes on a brush that sculpts or smooths",
    ),
    (
        SMOOTH,
        "[paint]\nchannel = \"color\"\ncolor_strength = 1.5",
        "paint.color_strength",
    ),
    (
        MOVE,
        "[stroke]\nclass = \"drag\"\n[paint]\nchannel = \"color\"",
        "[paint] needs a stroke that lays dabs",
    ),
    (
        "type = \"attribute\"\ngain = 1\ncolumn = \"mask\"\nvalue = 1.5",
        "",
        "kernel.value must be a number from 0 to 1",
    ),
    (
        "type = \"attribute\"\ngain = 1\ncolumn = \"hide\"\nvalue = 0.5",
        "",
        "kernel.value must be a whole number from 0 to 1",
    ),
    (
        "type = \"attribute\"\ngain = 1\ncolumn = \"group\"\nvalue = -1",
        "",
        "kernel.value must be a whole number",
    ),
    (
        "type = \"attribute\"\ngain = 1\ncolumn = \"group\"\nvalue = 4294967296",
        "",
        "kernel.value must be a whole number",
    ),
    (SMOOTH, "[tip]\nhardness = 1", "tip.hardness"),
    (SMOOTH, "[tip]\nfocal = -101", "tip.focal"),
    (
        SMOOTH,
        "[tip]\nprofile = \"custom\"",
        "tip.points is required",
    ),
    (
        SMOOTH,
        "[tip]\npoints = [[0, 1], [1, 0]]",
        "tip.points is required",
    ),
    (
        SMOOTH,
        "[tip]\nprofile = \"custom\"\npoints = [[0, 1], [0, 0.5], [1, 0]]",
        "rising x",
    ),
    (
        SMOOTH,
        "[tip]\nprofile = \"custom\"\npoints = [[0, 1], [0.9, 0]]",
        "from x = 0 to x = 1",
    ),
    (
        SMOOTH,
        "[tip]\nprofile = \"custom\"\npoints = [[0.1, 1], [1, 0]]",
        "from x = 0 to x = 1",
    ),
    (SMOOTH, "[pressure]\nsize = []", "at least two points"),
    (
        SMOOTH,
        "[pressure]\nstrength = [[0, -1], [1, 1]]",
        "pressure.strength values must be from 0 to 1",
    ),
    (
        SMOOTH,
        "[sampling]\nnormal_radius = 0",
        "sampling.normal_radius",
    ),
    (
        SMOOTH,
        "[sampling]\nposition_radius = 1.5",
        "sampling.position_radius",
    ),
    (
        SMOOTH,
        "[stroke]\nclass = \"dab\"\nspacing = 0",
        "stroke.spacing",
    ),
    (SMOOTH, "[stroke]\nclass = \"dab\"\ntap = 5", "stroke.tap"),
    (
        SMOOTH,
        "[stroke]\nclass = \"dab\"\nsmoothing = { radius = 5 }",
        "stroke.smoothing.radius",
    ),
    (
        SMOOTH,
        "[stroke]\nclass = \"dab\"\nsmoothing = { factor = 1 }",
        "stroke.smoothing.factor",
    ),
    (
        SMOOTH,
        "[stroke]\nclass = \"anchored\"\ntap = -1",
        "stroke.tap",
    ),
    (
        SMOOTH,
        "[stroke]\nclass = \"curve\"\nspacing = 9",
        "stroke.spacing",
    ),
    (
        MOVE,
        "[stroke]\nclass = \"drag\"\nsmoothing = { radius = 9 }",
        "stroke.smoothing.radius",
    ),
    (MOVE, "", "stroke.class does not fit"),
    (SMOOTH, DRAG, "stroke.class does not fit"),
    (
        "type = \"hook\"",
        "[stroke]\nclass = \"curve\"",
        "stroke.class does not fit",
    ),
    (
        SMOOTH,
        "[stroke]\nclass = \"gesture\"\nshape = \"curve\"",
        "stroke.class does not fit",
    ),
    (
        "type = \"plane\"\ngain = 1",
        "[stroke]\nclass = \"gesture\"\nshape = \"polygon\"",
        "stroke.class does not fit",
    ),
    (
        "type = \"attribute\"\ngain = 1\ncolumn = \"hide\"\nvalue = 1",
        "[stroke]\nclass = \"gesture\"\nshape = \"curve\"",
        "stroke.class does not fit",
    ),
    (
        "type = \"attribute\"\ngain = 1\ncolumn = \"mask\"",
        "[stroke]\nclass = \"gesture\"\nshape = \"polygon\"",
        "stroke.class does not fit",
    ),
    (SMOOTH, "[post]\nauto_smooth = -1", "post.auto_smooth"),
    (SMOOTH, "[post]\ngravity = 2", "post.gravity"),
    (
        SMOOTH,
        "[post]\ngravity_dir = [0, 0, 0]",
        "post.gravity_dir",
    ),
    (
        SMOOTH,
        "[post]\ngravity_dir = [0, inf, 0]",
        "post.gravity_dir",
    ),
    (SMOOTH, "[post]\ntopology_rake = 0.5", "not built yet"),
    (SMOOTH, "[live_detail]\nsize = 0", "live_detail.size"),
    (SMOOTH, "[live_detail]\nmerge = 2", "live_detail.merge"),
    (
        SMOOTH,
        "[live_detail]\nmode = \"pixels\"",
        "unknown variant",
    ),
    (SMOOTH, "[automask]\nbackface = 2", "automask.backface"),
    (SMOOTH, "[automask]\ncavity = -2", "automask.cavity"),
    (SMOOTH, "[automask]\ntopology = 17", "automask.topology"),
    (
        SMOOTH,
        "[automask]\nface_group = 1.5",
        "automask.face_group",
    ),
    (SMOOTH, "[automask]\nbogus = 1", "unknown field `bogus`"),
];

/// (kernel table, other tables): presets at the edges of every range, all valid.
const GOOD: &[(&str, &str)] = &[
    (
        SMOOTH,
        "[tip]\nprofile = \"custom\"\npoints = [[0, 1], [1, 0]]\nhardness = 0.99\nfocal = 100",
    ),
    (
        SMOOTH,
        "[tip]\nfocal = -100\n[tip.alpha]\nimage = \"grain\"",
    ),
    (
        SMOOTH,
        "[sampling]\nnormal_radius = 0.01\nposition_radius = 1\naccumulate = false\nfront_only = false",
    ),
    (SMOOTH, "[stroke]\nclass = \"dab\"\nspacing = 0.01\ntap = 0"),
    (
        SMOOTH,
        "[stroke]\nclass = \"anchored\"\nfollow = true\ntap = 4",
    ),
    (
        SMOOTH,
        "[stroke]\nclass = \"curve\"\nspacing = 0.01\nstraight = true",
    ),
    (
        SMOOTH,
        "[post]\nauto_smooth = 100\ngravity = -1\ngravity_dir = [0, 0, -3]",
    ),
    (
        SMOOTH,
        "[automask]\nbackface = 1\ncavity = 1\ntopology = 16\nface_group = 1",
    ),
    (SMOOTH, "[automask]\ncavity = -1\ntopology = 0"),
    (
        "type = \"offset\"\nterms = [{ dir = \"to_center\", gain = -4, exponent = 0.25, tangent = true }, { dir = \"view\", gain = 4, exponent = 16 }]",
        "",
    ),
    (
        "type = \"plane\"\ngain = 100\nheight = 16\ndepth = 0\nsource = \"stroke_normal\"",
        "[depth]\nimbed = -1",
    ),
    (
        "type = \"plane\"\ngain = 1",
        "[depth]\nimbed = 1\n[pressure]\nimbed = [[0, 0], [1, 1]]",
    ),
    (
        "type = \"plane\"\ngain = 0",
        "[stroke]\nclass = \"gesture\"\nshape = \"curve\"",
    ),
    (
        SMOOTH,
        "[tip.alpha]\nimage = \"blake3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"\nmid = 1\nflip = \"both\"\nturn = 3\ninvert = true\nmaximize = true\nblur = 64\nnoise = 1\nnoise_scale = 1024\nnoise_seed = 4294967295\ncontrast = 16\ncurve = { points = [[0, 0], [0.5, 0.2], [1, 1]], smooth = true }\nradial_fade = 8\ntiles = [64, 1]",
    ),
    (
        SMOOTH,
        "[tip.alpha]\nimage = \"square\"\ncontrast = 0\ntiles = [1, 64]",
    ),
    (
        SMOOTH,
        "[pressure]\nsize = { points = [[0, 0], [0.3, 0.6], [1, 1]], smooth = true, sharp = [0, 1, 2] }\nmodifier = [[0, 0], [1, 1]]\nglobal = true",
    ),
    (
        SMOOTH,
        "[post]\nauto_smooth = 1\nauto_smooth_mode = \"relax\"\nauto_smooth_max_faces = 4294967295",
    ),
    (
        "type = \"move\"\nselect = \"elastic\"\nsupport = 16\npoisson = 0.5",
        "[stroke]\nclass = \"drag\"\nsmoothing = { radius = 4, factor = 0.99 }",
    ),
    (
        "type = \"move\"\nselect = \"connected\"\nsupport = 1\npoisson = 0",
        DRAG,
    ),
    (
        "type = \"hook\"\npinch = 1",
        "[stroke]\nclass = \"dab\"\nspacing = 4\ntap = 4",
    ),
    (
        "type = \"attribute\"\ngain = 100\ncolumn = \"mask\"\nop = \"blur\"\nvalue = 1",
        "",
    ),
    (
        "type = \"attribute\"\ngain = 0\ncolumn = \"mask\"\nop = \"sharpen\"\nvalue = 0",
        "",
    ),
    (
        "type = \"attribute\"\ngain = 1\ncolumn = \"group\"\nvalue = 4294967295",
        "",
    ),
    (
        "type = \"attribute\"\ngain = 1\ncolumn = \"group\"\nvalue = 7",
        "",
    ),
    (SMOOTH, "[paint]\nchannel = \"color\"\ncolor_strength = 0"),
    (
        "type = \"offset\"\nterms = [{ dir = \"x\", gain = 1 }]",
        "[stroke]\nclass = \"curve\"\n[paint]\nchannel = \"color\"\ncolor_strength = 1",
    ),
    (
        "type = \"attribute\"\ngain = 1\ncolumn = \"hide\"\nvalue = 1",
        "[stroke]\nclass = \"gesture\"\nshape = \"polygon\"",
    ),
];

fn refused(text: &str, want: &str) {
    match BrushPreset::from_toml(text) {
        Ok(_) => panic!("accepted:\n{text}"),
        Err(e) => assert!(
            e.to_string().contains(want),
            "{e:?} should say {want:?}\nfor:\n{text}"
        ),
    }
}

#[test]
fn mistakes_are_refused_with_the_key_named() {
    for (text, want) in BAD_FILES {
        refused(text, want);
    }
    for (kernel, rest, want) in BAD {
        refused(&with(kernel, rest), want);
    }
}

#[test]
fn every_edge_value_the_format_allows_is_accepted() {
    for strength in ["0", "1"] {
        let text = with(SMOOTH, "").replace("strength = 0.5", &format!("strength = {strength}"));
        assert!(BrushPreset::from_toml(&text).is_ok(), "strength {strength}");
    }
    for (kernel, rest) in GOOD {
        let text = with(kernel, rest);
        if let Err(e) = BrushPreset::from_toml(&text) {
            panic!("refused ({e}):\n{text}");
        }
    }
    // Mode off: a brush that only paints.
    let text = with(SMOOTH, "[paint]\nchannel = \"color\"")
        .replace("strength = 0.5\n", "strength = 0.5\nmode = \"off\"\n");
    if let Err(e) = BrushPreset::from_toml(&text) {
        panic!("refused ({e}):\n{text}");
    }
}

/// (short form, the same with every left-out key written as its documented default).
const DEFAULTS: &[(&str, &str, &str, &str)] = &[
    (
        SMOOTH,
        "",
        SMOOTH,
        "[tip]\nprofile = \"smooth\"\nhardness = 0\nfocal = 0\n[depth]\nimbed = 0\n[sampling]\nnormal_radius = 0.5\nposition_radius = 0.5\naccumulate = true\nfront_only = true\n[stroke]\nclass = \"dab\"\nspacing = 0.1\ntap = 0.5\nsmoothing = { radius = 0, factor = 0 }\n[pressure]\nsize = [[0, 1], [1, 1]]\nstrength = [[0, 0], [1, 1]]\nspacing = [[0, 1], [1, 1]]\nmodifier = [[0, 1], [1, 1]]\nimbed = [[0, 1], [1, 1]]\nglobal = false\n[post]\nauto_smooth = 0\nauto_smooth_mode = \"uniform\"\nauto_smooth_max_faces = 0\ngravity = 0\ngravity_dir = [0, -1, 0]\ntopology_rake = 0\n[automask]\nbackface = 0\ncavity = 0\ntopology = 0\nface_group = 0",
    ),
    (
        SMOOTH,
        "[tip.alpha]\nimage = \"ring\"",
        SMOOTH,
        "[tip.alpha]\nimage = \"ring\"\nmid = 0\nflip = \"none\"\nturn = 0\ninvert = false\nmaximize = false\nblur = 0\nnoise = 0\nnoise_scale = 16\nnoise_seed = 0\ncontrast = 1\ncurve = [[0, 0], [1, 1]]\nradial_fade = 0\ntiles = [1, 1]",
    ),
    (
        SMOOTH,
        "[pressure]\nsize = { points = [[0, 1], [1, 1]], smooth = false }",
        SMOOTH,
        "[pressure]\nsize = { points = [[0, 1], [1, 1]], smooth = true, sharp = [0, 1] }",
    ),
    (
        "type = \"move\"\nselect = \"ball\"",
        DRAG,
        "type = \"move\"\nselect = \"ball\"\nsupport = 4\npoisson = 0.45",
        "[stroke]\nclass = \"drag\"\nsmoothing = { radius = 0, factor = 0 }",
    ),
    (
        "type = \"offset\"\nterms = [{ dir = \"x\", gain = 1 }]",
        "",
        "type = \"offset\"\nterms = [{ dir = \"x\", gain = 1, exponent = 1, tangent = false }]",
        "",
    ),
    (
        "type = \"plane\"\ngain = 1",
        "",
        "type = \"plane\"\ngain = 1\nsource = \"dab\"\nside = \"both\"",
        "",
    ),
    ("type = \"hook\"", "", "type = \"hook\"\npinch = 0", ""),
    (
        "type = \"attribute\"\ncolumn = \"mask\"\ngain = 1",
        "",
        "type = \"attribute\"\ncolumn = \"mask\"\ngain = 1\nop = \"set\"\nvalue = 0",
        "",
    ),
    (
        SMOOTH,
        "[stroke]\nclass = \"anchored\"",
        SMOOTH,
        "[stroke]\nclass = \"anchored\"\nfollow = false\ntap = 0.5",
    ),
    (
        SMOOTH,
        "[stroke]\nclass = \"curve\"",
        SMOOTH,
        "[stroke]\nclass = \"curve\"\nspacing = 0.1\nstraight = false",
    ),
    (
        SMOOTH,
        "[stroke]\nclass = \"dab\"",
        SMOOTH,
        "[stroke]\nclass = \"dab\"\nspacing = 0.1\ntap = 0.5",
    ),
    (
        SMOOTH,
        "[paint]\nchannel = \"color\"",
        SMOOTH,
        "[paint]\nchannel = \"color\"\ncolor_strength = 1",
    ),
];

#[test]
fn a_left_out_key_means_its_documented_default() {
    let parse =
        |text: &str| BrushPreset::from_toml(text).unwrap_or_else(|e| panic!("{e}:\n{text}"));
    for (short_kernel, short_rest, long_kernel, long_rest) in DEFAULTS {
        let (short, long) = (
            parse(&with(short_kernel, short_rest)),
            parse(&with(long_kernel, long_rest)),
        );
        assert_eq!(
            short,
            long,
            "defaults of:\n{}",
            with(short_kernel, short_rest)
        );
        assert!(short.claims.is_empty(), "no claims");
    }
    let plain = with(SMOOTH, "");
    let top = plain.replace(
        "strength = 0.5\n",
        "strength = 0.5\nmodifier = 0\nmode = \"add\"\n",
    );
    assert_eq!(parse(&plain), parse(&top), "modifier 0 and mode add");
    assert!(parse(&plain).tip.alpha.is_none(), "no alpha");
}

#[test]
fn a_patch_merges_over_the_preset_and_is_checked_like_a_file() {
    let text = with(
        SMOOTH,
        "[tip]\nhardness = 0.3\nfocal = 10\n[stroke]\nclass = \"dab\"\nspacing = 0.2",
    );
    let table = |s: &str| ok(toml::from_str::<gs_core::brush::Patch>(s));
    let resolved = ok(BrushPreset::resolve(
        &text,
        &table("strength = 0.9\n[tip]\nfocal = -40\n[tip.alpha]\nimage = \"ring\"\nmid = 0.5"),
    ));
    // The patch's keys win, every other key stays, nested tables merge key by key.
    let mut want = ok(BrushPreset::from_toml(&text));
    want.strength = 0.9;
    want.tip.focal = -40.0;
    want.tip.alpha = Some(gs_core::brush::AlphaSpec {
        mid: 0.5,
        ..gs_core::brush::AlphaSpec::plain("ring")
    });
    assert_eq!(resolved, want, "merged key by key");
    assert_eq!(
        ok(BrushPreset::resolve(&text, &table(""))),
        ok(BrushPreset::from_toml(&text)),
        "an empty patch is the preset"
    );
    for (patch, says) in [
        ("[tip]\nfocal = 200", "tip.focal must be from -100 to 100"),
        ("name = \"Other\"", "settings cannot change name"),
        ("family = \"Mask\"", "settings cannot change family"),
        (
            "[kernel]\ntype = \"plane\"",
            "settings cannot change kernel.type",
        ),
        ("[tip]\nbogus = 1", "unknown field `bogus`"),
        (
            "modifier = 30",
            "modifier does nothing on the smooth kernel",
        ),
    ] {
        let e = BrushPreset::resolve(&text, &table(patch)).err();
        let e = e.map(|e| e.to_string()).unwrap_or_default();
        assert!(e.contains(says), "{patch}: {e}");
    }
}

#[test]
fn a_patch_of_the_wrong_shape_is_refused_at_its_key_not_a_line_of_the_preset() {
    // Review 0004, m2: the merged table is read as it stands, so the refusal names the key in
    // the settings, never a line of the merged text, and says a list replaces the whole list.
    let draw = "type = \"offset\"\nterms = [{ dir = \"area_normal\", gain = 0.5 }]";
    let text = with(draw, "");
    let table = |s: &str| ok(toml::from_str::<gs_core::brush::Patch>(s));
    for (patch, says) in [
        (
            "[[kernel.terms]]\ngain = 1",
            "kernel: missing field `dir`; settings.kernel.terms replaces the preset's whole \
             list, so write each item in full",
        ),
        (
            "strength = \"high\"",
            "strength: invalid type: string \"high\", expected f32",
        ),
        (
            "[tip]\nfocul = 5",
            "tip.focul: unknown field `focul`, expected one of",
        ),
    ] {
        let e = BrushPreset::resolve(&text, &table(patch)).err();
        let e = e.map(|e| e.message).unwrap_or_default();
        assert!(e.starts_with(says), "{patch}: {e}");
        assert!(
            !e.contains("line") && !e.contains("preset does not parse"),
            "{e}"
        );
    }
}

#[test]
fn a_curve_point_above_1_is_refused_naming_the_setting() {
    // Review 0004, M3: every curve is read as a fraction, so a point on a 0 to 100 scale is
    // refused by the setting's name, in a file and in a patch, not run as a 100-fold dab.
    let text = with(SMOOTH, "");
    let table = |s: &str| ok(toml::from_str::<gs_core::brush::Patch>(s));
    let message = |r: Result<BrushPreset, gs_core::PresetError>| r.err().map(|e| e.message);
    for (rest, path) in [
        (
            "[tip]\nprofile = \"custom\"\npoints = [[0, 100], [1, 100]]",
            "tip.points",
        ),
        (
            "[tip.alpha]\nimage = \"ring\"\ncurve = [[0, 0], [1, 2]]",
            "tip.alpha.curve",
        ),
        (
            "[pressure]\nstrength = [[0, 100], [1, 100]]",
            "pressure.strength",
        ),
        (
            "[pressure]\nsize = { points = [[0, 1], [0.5, 1.5], [1, 1]], smooth = true }",
            "pressure.size",
        ),
        ("[pressure]\nimbed = [[0, 0], [1, 1.01]]", "pressure.imbed"),
    ] {
        let says = format!("{path} values must be from 0 to 1");
        let file = message(BrushPreset::from_toml(&with(SMOOTH, rest))).unwrap_or_default();
        assert!(file.contains(&says), "file {rest}: {file}");
        let patch = message(BrushPreset::resolve(&text, &table(rest))).unwrap_or_default();
        assert!(patch.contains(&says), "patch {rest}: {patch}");
    }
    let recipe = "name = \"R\"\ncatalog_id = \"r\"\n[[layer]]\nshape = \"profile\"\n\
                  points = [[0, 2], [1, 0]]";
    let e = gs_core::alpha::Recipe::from_toml(recipe).err();
    assert_eq!(
        e.as_deref(),
        Some("layer 0: profile values must be from 0 to 1")
    );
    // A custom tip built in code is held to 0 to 1 once, when it is built.
    let loud = ok(gs_core::curve::Curve::new(vec![[0.0, 100.0], [1.0, 100.0]]));
    let tip = gs_core::falloff::Tip::new(gs_core::falloff::Profile::Custom(loud), 0.0, 0.0);
    assert_eq!(tip.falloff(0.5), 1.0, "a falloff never leaves 0 to 1");
}

#[test]
fn the_canonical_form_reads_back_and_the_hash_follows_the_meaning() {
    for s in crate::brush_kit::shipped() {
        let p = &s.preset;
        let canonical = p.canonical();
        assert_eq!(
            ok(BrushPreset::from_toml(&canonical)),
            *p,
            "{}: the canonical form reads back as the same preset",
            s.stem
        );
        // Comments and layout do not count.
        let reworded = format!("# another comment\n\n{}", s.text.replace(" = ", "="));
        assert_eq!(
            ok(BrushPreset::from_toml(&reworded)).hash(),
            p.hash(),
            "{}",
            s.stem
        );
        let changed = BrushPreset {
            strength: p.strength * 0.5 + 0.01,
            ..p.clone()
        };
        assert_ne!(changed.hash(), p.hash(), "{}: a changed preset", s.stem);
        // A Live Detail table that only says the defaults is no table, so every preset kept
        // its hash when the table came (decision 0021, item 11.8); an opt-out is a change.
        let said = if s.text.contains("[live_detail]\n") {
            (s.text).replacen("[live_detail]\n", "[live_detail]\nenable = true\n", 1)
        } else {
            format!("{}\n[live_detail]\nenable = true\n", s.text)
        };
        assert_eq!(
            ok(BrushPreset::from_toml(&said)).hash(),
            p.hash(),
            "{}",
            s.stem
        );
        let mut off = p.clone();
        off.live_detail.enable = false;
        assert_ne!(off.hash(), p.hash(), "{}: a preset that opts out", s.stem);
        assert_eq!(p.hash().len(), 64, "64 hex digits");
    }
}
