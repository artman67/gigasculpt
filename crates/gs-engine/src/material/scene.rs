//! The material palette in a scene file (decision 0043, section 7). The scene section's
//! `palette` keeps every material in full and the id counter, the session section's `material`
//! the active one. Opening a scene replaces the session's palette with the file's, or with the
//! shipped one, MatCap Gray active, when the file keeps none (format 4 or older, or a Part file)
//! or one this version cannot read; the user folders the session read go into that one once the
//! open is logged (`Engine::read_folders_again`).

use std::sync::Arc;

use gs_core::material::Recipe;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::palette::{LATER, PaletteEntry, PaletteState};

/// The palette as a scene file keeps it.
#[derive(Serialize, Deserialize)]
struct Kept {
    /// Ids handed out so far; the next material gets a later one.
    issued: u64,
    /// In palette order.
    materials: Vec<KeptMaterial>,
}

/// One material as a scene file keeps it.
#[derive(Serialize, Deserialize)]
struct KeptMaterial {
    id: u64,
    name: String,
    built_in: bool,
    /// Its title, kind, environment, physical keys, bake and cavity settings, colors linear.
    recipe: Recipe,
}

/// The scene section's `palette` for `palette`.
pub(crate) fn section(palette: &PaletteState) -> Value {
    let kept = Kept {
        issued: palette.issued,
        materials: (palette.now.materials.iter())
            .map(|m| KeptMaterial {
                id: m.id,
                name: m.name.clone(),
                built_in: m.built_in,
                recipe: (*m.recipe).clone(),
            })
            .collect(),
    };
    serde_json::to_value(kept).unwrap_or_default()
}

/// The palette a scene file opens with, from its scene section's `palette` and its session
/// section's active material, with no history. When the file keeps a palette this version
/// cannot read, the shipped one, and why, for the open's `skipped`.
pub(crate) fn opened(
    section: Option<Value>,
    active: Option<u64>,
) -> (PaletteState, Option<String>) {
    let bare = || {
        let mut shipped = PaletteState::default();
        shipped.bare = true;
        shipped
    };
    let Some(section) = section else {
        return (bare(), None);
    };
    match read(section, active) {
        Ok(palette) => (palette, None),
        Err(why) => {
            let said = format!("the material palette ({why}): the shipped one opened instead");
            (bare(), Some(said))
        }
    }
}

/// Reads a kept palette: every recipe checked as a look file is, ids and names unique, at
/// least one material. A counter below an id the file holds (outside the later built-ins'
/// block, which no new material takes) is raised to it, so no id is handed out twice.
fn read(section: Value, active: Option<u64>) -> Result<PaletteState, String> {
    let kept: Kept = serde_path_to_error::deserialize(section)
        .map_err(|e| format!("{}: {}", e.path(), e.inner()))?;
    let mut palette = PaletteState::default();
    palette.now.materials.clear();
    palette.issued = kept.issued;
    for m in kept.materials {
        let at = format!("material #{} '{}'", m.id, m.name);
        m.recipe.check().map_err(|e| format!("{at}: {e}"))?;
        if palette.now.materials.iter().any(|o| o.id == m.id) {
            return Err(format!("two materials are #{}", m.id));
        }
        let named = palette.now.check_name(&m.name, Some(m.id));
        named.map_err(|e| format!("{at}: {}", e.message))?;
        if !LATER.contains(&m.id) {
            palette.issued = palette.issued.max(m.id);
        }
        palette.now.materials.push(PaletteEntry {
            id: m.id,
            name: m.name,
            built_in: m.built_in,
            recipe: Arc::new(m.recipe),
        });
    }
    if palette.now.materials.is_empty() {
        return Err("it holds no material".into());
    }
    if let Some(id) = active {
        palette.now.active = id;
    }
    palette.now.active = palette.now.active().id;
    Ok(palette)
}
