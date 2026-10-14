//! The library of looks: recipe files read into named looks. A file that does not read is
//! refused, with the file and the key it got wrong named, and the rest still load. Also the
//! shipped environments a recipe may name.

use super::Recipe;

/// Every shipped look recipe (`assets/matcaps/`), built in, in the order they shipped: MatCap
/// Gray and MatCap Red Wax first (catalog item `material.palette`), then the catalog's order,
/// then (decision 0043, step (c)) Flat Color and the six standard materials. A new recipe goes
/// at the end, so the ids and places of the ones before it stay.
pub const SHIPPED: [(&str, &str); 20] = [
    (
        "assets/matcaps/gray.toml",
        include_str!("../../../../assets/matcaps/gray.toml"),
    ),
    (
        "assets/matcaps/red-wax.toml",
        include_str!("../../../../assets/matcaps/red-wax.toml"),
    ),
    (
        "assets/matcaps/white.toml",
        include_str!("../../../../assets/matcaps/white.toml"),
    ),
    (
        "assets/matcaps/red-clay.toml",
        include_str!("../../../../assets/matcaps/red-clay.toml"),
    ),
    (
        "assets/matcaps/light-brown-clay.toml",
        include_str!("../../../../assets/matcaps/light-brown-clay.toml"),
    ),
    (
        "assets/matcaps/green-clay.toml",
        include_str!("../../../../assets/matcaps/green-clay.toml"),
    ),
    (
        "assets/matcaps/bone.toml",
        include_str!("../../../../assets/matcaps/bone.toml"),
    ),
    (
        "assets/matcaps/white-cavity.toml",
        include_str!("../../../../assets/matcaps/white-cavity.toml"),
    ),
    (
        "assets/matcaps/skin-4.toml",
        include_str!("../../../../assets/matcaps/skin-4.toml"),
    ),
    (
        "assets/matcaps/metal-1.toml",
        include_str!("../../../../assets/matcaps/metal-1.toml"),
    ),
    (
        "assets/matcaps/chrome-a.toml",
        include_str!("../../../../assets/matcaps/chrome-a.toml"),
    ),
    (
        "assets/matcaps/sketch.toml",
        include_str!("../../../../assets/matcaps/sketch.toml"),
    ),
    (
        "assets/matcaps/sketch-shaded-1.toml",
        include_str!("../../../../assets/matcaps/sketch-shaded-1.toml"),
    ),
    (
        "assets/matcaps/flat-color.toml",
        include_str!("../../../../assets/matcaps/flat-color.toml"),
    ),
    (
        "assets/matcaps/fast-shader.toml",
        include_str!("../../../../assets/matcaps/fast-shader.toml"),
    ),
    (
        "assets/matcaps/basic-material.toml",
        include_str!("../../../../assets/matcaps/basic-material.toml"),
    ),
    (
        "assets/matcaps/basic-material-2.toml",
        include_str!("../../../../assets/matcaps/basic-material-2.toml"),
    ),
    (
        "assets/matcaps/toy-plastic.toml",
        include_str!("../../../../assets/matcaps/toy-plastic.toml"),
    ),
    (
        "assets/matcaps/reflected-map.toml",
        include_str!("../../../../assets/matcaps/reflected-map.toml"),
    ),
    (
        "assets/matcaps/soft-skin-shader.toml",
        include_str!("../../../../assets/matcaps/soft-skin-shader.toml"),
    ),
];

/// The look a session starts with (ARCHITECTURE 6: MatCap Gray).
pub const DEFAULT_LOOK: &str = "gray";

/// Every shipped environment (`assets/environments/`), built in: `(name, file text)`. A
/// recipe's `environment` names one; gs-render builds them into light.
pub const ENVIRONMENTS: [(&str, &str); 5] = [
    (
        "studio",
        include_str!("../../../../assets/environments/studio.toml"),
    ),
    (
        "soft-studio",
        include_str!("../../../../assets/environments/soft-studio.toml"),
    ),
    (
        "studio-small-09",
        include_str!("../../../../assets/environments/studio-small-09.toml"),
    ),
    (
        "studio-small-08",
        include_str!("../../../../assets/environments/studio-small-08.toml"),
    ),
    (
        "photo-studio-01",
        include_str!("../../../../assets/environments/photo-studio-01.toml"),
    ),
];

/// One look: its name (the recipe file's stem, such as `red-wax`) and its recipe.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub name: String,
    pub recipe: Recipe,
}

/// A recipe file that was not read, and why: the key it got wrong and what is wrong with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    pub file: String,
    pub error: String,
}

/// Looks read from recipe files, in the order the files came.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Library {
    pub looks: Vec<Look>,
    pub refused: Vec<Refused>,
}

impl Library {
    /// The shipped looks.
    pub fn shipped() -> Library {
        Library::read(&SHIPPED)
    }

    /// Reads `(file, text)` pairs in order. Each look is named by its file's stem; a file whose
    /// stem an earlier file already took is refused.
    pub fn read(files: &[(&str, &str)]) -> Library {
        let mut library = Library::default();
        for &(file, text) in files {
            let name = stem(file);
            let read = if library.get(name).is_some() {
                Err(format!("the name {name} is taken by an earlier file"))
            } else {
                Recipe::parse(text).map_err(|e| format!("recipe: {e}"))
            };
            match read {
                Ok(recipe) => library.looks.push(Look {
                    name: name.to_string(),
                    recipe,
                }),
                Err(error) => library.refused.push(Refused {
                    file: file.to_string(),
                    error,
                }),
            }
        }
        library
    }

    /// The look called `name`.
    pub fn get(&self, name: &str) -> Option<&Look> {
        self.looks.iter().find(|l| l.name == name)
    }
}

/// The file name without its folders and extension.
fn stem(file: &str) -> &str {
    let name = file.rsplit(['/', '\\']).next().unwrap_or(file);
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}
