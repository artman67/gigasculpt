//! Alphas (decision 0018): the library of pictures brushes stamp with, the cache of processed
//! pyramids, and the `alpha.*` commands.
//!
//! The library is the shipped recipes in `assets/alphas/`, built in like the brush presets and
//! drawn the first time a picture is used. A brush names a picture by its library name or by
//! `blake3:` and its hash. Processed pyramids are cached by picture hash and tone hash in one
//! ordered map with a byte budget; a picture or pyramid the cache dropped is made again with the
//! same bytes. User pictures, imports and exports come with phase B4. The session keeps the
//! alphas used lately for the Alpha palette (`recent`).

mod list;
mod preview;
mod recent;

pub(crate) use recent::Recent;

/// Bits per pixel of every alpha: 16-bit grey, so gradients have no steps.
pub(crate) const BITS: u32 = 16;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use gs_core::alpha::{Picture, Pyramid, Recipe};
use gs_core::brush::AlphaSpec;
use gs_protocol::CommandError;

use crate::errors;

/// Every shipped recipe file, by library name. A new one goes last, so every alpha keeps its
/// index (decision 0051).
const FILES: [(&str, &str); 7] = [
    (
        "grainy-round",
        include_str!("../../../../assets/alphas/grainy-round.toml"),
    ),
    (
        "hard-round",
        include_str!("../../../../assets/alphas/hard-round.toml"),
    ),
    ("ring", include_str!("../../../../assets/alphas/ring.toml")),
    (
        "soft-round",
        include_str!("../../../../assets/alphas/soft-round.toml"),
    ),
    (
        "square",
        include_str!("../../../../assets/alphas/square.toml"),
    ),
    (
        "white",
        include_str!("../../../../assets/alphas/white.toml"),
    ),
    (
        "checker",
        include_str!("../../../../assets/alphas/checker.toml"),
    ),
];

/// The most bytes the cache of processed pyramids keeps.
const BUDGET: usize = 512 << 20;

/// One shipped alpha: its name, recipe file and recipe, and the picture once drawn.
pub(crate) struct Shipped {
    pub name: &'static str,
    pub text: &'static str,
    pub recipe: Recipe,
    picture: OnceLock<Result<Arc<Picture>, String>>,
}

impl Shipped {
    /// The picture its recipe draws, drawn the first time it is asked for.
    pub fn picture(&self) -> Result<Arc<Picture>, CommandError> {
        let drawn = self
            .picture
            .get_or_init(|| self.recipe.draw().map(Arc::new));
        drawn
            .clone()
            .map_err(|e| errors::internal(&format!("drawing the alpha {}: {e}", self.name)))
    }
}

/// Every shipped alpha, by name. A recipe that does not read is left out (the tests read every
/// one, so none is).
pub(crate) fn shipped() -> &'static [Shipped] {
    static ALL: OnceLock<Vec<Shipped>> = OnceLock::new();
    ALL.get_or_init(|| {
        (FILES.iter())
            .filter_map(|&(name, text)| {
                let recipe = Recipe::from_toml(text).ok()?;
                Some(Shipped {
                    name,
                    text,
                    recipe,
                    picture: OnceLock::new(),
                })
            })
            .collect()
    })
}

/// The shipped alpha called `name` (any case). `E_NOT_FOUND` with the closest names otherwise.
pub(crate) fn find(name: &str) -> Result<&'static Shipped, CommandError> {
    let all = shipped();
    let wanted = name.trim();
    (all.iter())
        .find(|s| s.name.eq_ignore_ascii_case(wanted))
        .ok_or_else(|| CommandError {
            hint: Some("alpha.list lists every alpha".into()),
            ..errors::not_found("alpha", name, all.iter().map(|s| s.name))
        })
}

/// The picture a brush names: a library name, or `blake3:` and the hash of a library picture.
pub(crate) fn picture(image: &str) -> Result<Arc<Picture>, CommandError> {
    let Some(hash) = image.strip_prefix("blake3:") else {
        return find(image)?.picture();
    };
    for s in shipped() {
        let p = s.picture()?;
        if p.hash() == hash {
            return Ok(p);
        }
    }
    Err(CommandError {
        hint: Some("alpha.list lists every alpha with its hash".into()),
        ..errors::error(
            gs_protocol::ErrorCode::NotFound,
            format!("no alpha in the library has the hash {hash}"),
        )
    })
}

/// The processed pyramids, by picture hash and tone hash, each with when it was last used.
#[derive(Default)]
struct Cache {
    map: BTreeMap<(String, String), (u64, Arc<Pyramid>)>,
    clock: u64,
    bytes: usize,
}

/// The pyramid a brush using an alpha the way `spec` says reads: from the cache, or built (a
/// pure function of picture and tone settings, so a rebuilt one has the same bytes).
pub(crate) fn pyramid(spec: &AlphaSpec) -> Result<Arc<Pyramid>, CommandError> {
    static CACHE: Mutex<Cache> = Mutex::new(Cache {
        map: BTreeMap::new(),
        clock: 0,
        bytes: 0,
    });
    let picture = picture(&spec.image)?;
    let key = (picture.hash().to_string(), spec.tone_key());
    {
        let mut c = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
        c.clock += 1;
        let now = c.clock;
        if let Some((used, p)) = c.map.get_mut(&key) {
            *used = now;
            return Ok(Arc::clone(p));
        }
    }
    let built = Arc::new(Pyramid::build(&picture, spec));
    let mut c = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    // Another session may have built the same one meanwhile: the bytes are the same.
    if let Some((_, p)) = c.map.get(&key) {
        return Ok(Arc::clone(p));
    }
    while c.bytes + built.bytes() > BUDGET {
        let oldest = (c.map.iter()).min_by_key(|e| (e.1).0).map(|e| e.0.clone());
        let Some(gone) = oldest.and_then(|k| c.map.remove(&k)) else {
            break;
        };
        c.bytes -= gone.1.bytes();
    }
    c.bytes += built.bytes();
    let now = c.clock;
    c.map.insert(key, (now, Arc::clone(&built)));
    Ok(built)
}
