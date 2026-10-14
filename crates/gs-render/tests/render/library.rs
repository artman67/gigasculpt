//! The library of looks: every shipped recipe is in it, and a bad file is refused by name.

use crate::common::shipped_recipes;
use gs_core::material::MaterialKind;
use gs_render::display::encode;
use gs_render::matcap::{Bake, DEFAULT_LOOK, Library, Options, Recipe, bake_under};
use gs_render::shading::Environment;

/// Every recipe file in `assets/matcaps/` is built into the library and reads, so adding a
/// file without listing it in `SHIPPED` fails here.
#[test]
fn every_shipped_recipe_is_in_the_library() {
    let library = Library::shipped();
    assert!(
        library.refused.is_empty(),
        "shipped recipes all read: {:?}",
        library.refused
    );
    let mut ours: Vec<&str> = library.looks.iter().map(|l| l.name.as_str()).collect();
    ours.sort_unstable();
    let files = shipped_recipes();
    let mut on_disk: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
    on_disk.sort_unstable();
    assert_eq!(ours, on_disk, "the library holds exactly the files on disk");
    assert_eq!(
        library.looks.first().map(|l| l.name.as_str()),
        Some(DEFAULT_LOOK),
        "MatCap Gray comes first and is the default"
    );
}

/// The six standard built-ins (decision 0043, step (c)) follow Flat Color in the catalog's
/// order, each of kind standard, and each is a look of its own: baked small under its own
/// environment and seen with white paint (the albedo times D plus S, as sRGB at exposure 0),
/// any two differ by 3 levels or more in some channel on at least one pixel of the ball in 20.
#[test]
fn the_standard_built_ins_follow_flat_color_each_a_look_of_its_own() {
    const SIZE: u32 = 32;
    let on_ball = |p: u32| {
        let at = |i: u32| (i as f32 + 0.5) * 2.0 / SIZE as f32 - 1.0;
        let (u, v) = (at(p % SIZE), at(p / SIZE));
        u * u + v * v < 1.0
    };
    let library = Library::shipped();
    let later: Vec<&str> = (library.looks.iter().skip(13))
        .map(|l| l.name.as_str())
        .collect();
    assert_eq!(
        later,
        [
            "flat-color",
            "fast-shader",
            "basic-material",
            "basic-material-2",
            "toy-plastic",
            "reflected-map",
            "soft-skin-shader"
        ],
        "after the thirteen MatCaps, in the catalog's order"
    );
    let standard = &library.looks[14..];
    let pictures: Vec<Vec<[u8; 3]>> = (standard.iter())
        .map(|look| {
            assert_eq!(look.recipe.kind, MaterialKind::Standard, "{}", look.name);
            let small = Recipe {
                bake: Bake {
                    size: SIZE,
                    ..look.recipe.bake
                },
                ..look.recipe.clone()
            };
            let env = Environment::shipped(&small.environment).expect("its environment builds");
            let baked = bake_under(&small, &env, Options::default());
            let albedo = baked.albedo();
            (0..SIZE * SIZE)
                .filter(|&p| on_ball(p))
                .map(|p| {
                    let (d, s) = baked.layers(p % SIZE, p / SIZE);
                    encode([0, 1, 2].map(|k| albedo[k] * d[k] + s[k]))
                })
                .collect()
        })
        .collect();
    for (i, a) in pictures.iter().enumerate() {
        for (j, b) in pictures.iter().enumerate().skip(i + 1) {
            let seen = (a.iter().zip(b))
                .filter(|(x, y)| (0..3).any(|k| x[k].abs_diff(y[k]) >= 3))
                .count();
            assert!(
                seen * 20 >= a.len(),
                "{} and {} are told apart on {seen} of {} pixels",
                standard[i].name,
                standard[j].name,
                a.len()
            );
        }
    }
}

const GOOD: &str = "name = \"good\"\n";

/// A file that does not read is refused with its file and the key it got wrong; the others
/// still load.
#[test]
fn a_bad_recipe_is_refused_with_its_file_and_key() {
    let bad = [
        (
            "wrong type",
            "[material]\nspecular_roughness = \"x\"",
            "material.specular_roughness",
        ),
        ("unknown key", "shine = 1\n[material]", "shine"),
        (
            "bad color",
            "[material]\nbase_color = \"#12345\"",
            "material.base_color",
        ),
        (
            "out of physical range",
            "[material]\nspecular_ior = 0.5",
            "material.specular_ior",
        ),
        (
            "albedo above 1",
            "[material]\nbase_color = [1.2, 0.5, 0.5]",
            "material.base_color",
        ),
        (
            "unknown environment",
            "environment = \"moon\"\n[material]",
            "environment",
        ),
        ("bake size", "[material]\n[bake]\nsize = 8", "bake.size"),
    ];
    let texts: Vec<String> = (bad.iter())
        .map(|(_, extra, _)| format!("{GOOD}{extra}\n"))
        .collect();
    let good = format!("{GOOD}[material]\n");
    let mut files = vec![("looks/good.toml", good.as_str())];
    let names: Vec<String> = (0..bad.len())
        .map(|i| format!("looks/bad-{i}.toml"))
        .collect();
    files.extend(
        names
            .iter()
            .map(String::as_str)
            .zip(texts.iter().map(String::as_str)),
    );
    files.push(("other/good.toml", good.as_str()));
    let library = Library::read(&files);
    assert_eq!(
        library
            .looks
            .iter()
            .map(|l| l.name.as_str())
            .collect::<Vec<_>>(),
        ["good"],
        "the good file loads, once"
    );
    assert_eq!(
        library.refused.len(),
        bad.len() + 1,
        "every bad file and the second good.toml are refused: {:?}",
        library.refused
    );
    for ((what, _, key), refused) in bad.iter().zip(&library.refused) {
        assert!(
            refused.file.starts_with("looks/bad-") && refused.error.contains(key),
            "{what}: {refused:?} names its file and the key {key}"
        );
    }
}
