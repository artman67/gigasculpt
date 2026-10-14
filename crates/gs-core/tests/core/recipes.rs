//! A look recipe written out (decision 0043, section 7: a scene file keeps every material's
//! recipe in full): every shipped recipe, and an edited standard one, written in the look file
//! format read back the same, colors written linear.

use gs_core::material::{Color, Library, MaterialKind, Recipe};

/// `recipe` written as a look file, and that file's colors.
fn written(recipe: &Recipe) -> (String, toml::Table) {
    let text = toml::to_string(recipe).unwrap_or_else(|e| panic!("a recipe writes: {e}"));
    let table = (text.parse()).unwrap_or_else(|e| panic!("as TOML: {e}\n{text}"));
    (text, table)
}

#[test]
fn a_recipe_written_out_reads_back_the_same() {
    let library = Library::shipped();
    assert!(library.refused.is_empty(), "{:?}", library.refused);
    for look in &library.looks {
        let (text, _) = written(&look.recipe);
        assert_eq!(
            Recipe::parse(&text),
            Ok(look.recipe.clone()),
            "{}",
            look.name
        );
    }
    // An edited standard material: values no shipped file holds, its cavity settings included.
    let standard = (library.looks.iter())
        .find(|l| l.recipe.kind == MaterialKind::Standard)
        .expect("a standard built-in");
    let mut edited = standard.recipe.clone();
    edited.material.specular_roughness = 1.0 / 3.0;
    edited.material.base_color = Color([0.1, 0.2, 0.3]);
    edited.cavity.intensity = 1.7;
    edited.cavity.color = Color([0.25, 0.5, 1.0 / 7.0]);
    let (text, table) = written(&edited);
    let back = Recipe::parse(&text).expect("an edited recipe reads back");
    let bits = |r: &Recipe| {
        let m = &r.material;
        let colors = [m.base_color, r.cavity.color].map(|c| c.0.map(f32::to_bits));
        (
            m.specular_roughness.to_bits(),
            r.cavity.intensity.to_bits(),
            colors,
        )
    };
    assert_eq!(
        (back == edited, bits(&back)),
        (true, bits(&edited)),
        "{text}"
    );
    assert_eq!(table["kind"].as_str(), Some("standard"));
    let base = &table["material"]["base_color"];
    assert!(
        base.as_array().is_some_and(|rgb| rgb.len() == 3),
        "a color is written linear, as [r, g, b]: {base}"
    );
}
