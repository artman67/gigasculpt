//! Where golden pictures live and how they move: references, the pending queue, the replaced
//! references kept for revert and the digest, and the failure pictures of a check.
//!
//! Each picture is a pair of files with one name: `<scene>.<look>.png` and its manifest,
//! `<scene>.<look>.toml`, naming the scene, look, size, adapter, driver and wgpu version.

use std::path::{Path, PathBuf};

use gs_render::{Gpu, Image, Renderer, WGPU_VERSION};
use serde::{Deserialize, Serialize};

use super::compare::{Verdict, compare, diff_map};
use super::scene::{GoldenScene, SIZE, render};

/// The golden folders.
#[derive(Clone, Debug)]
pub struct Goldens {
    /// Checked in (protected): `scenes/`, `reference/`, `previous/`.
    pub root: PathBuf,
    /// Not checked in: `pending/`, `failures/`, review and digest sheets.
    pub work: PathBuf,
    /// The look recipes MatCap and lit scenes use.
    pub matcaps: PathBuf,
}

/// What a picture was rendered from and on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub scene: String,
    /// As written in the scene file: `zebra`, `clay`, `matcap:<name>` or `lit:<name>`.
    pub look: String,
    pub size: u32,
    pub adapter: String,
    pub driver: String,
    pub wgpu: String,
    /// What the mesh checks found wrong with the scene when it was rendered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub defects: Vec<String>,
}

impl Manifest {
    /// A manifest for a picture of `scene` in `look` rendered on `gpu`.
    pub fn new(gpu: &Gpu, scene: &str, look: &str, defects: Vec<String>) -> Manifest {
        Manifest {
            scene: scene.into(),
            look: look.into(),
            size: SIZE,
            adapter: gpu.adapter_name().into(),
            driver: gpu.driver(),
            wgpu: WGPU_VERSION.into(),
            defects,
        }
    }

    /// Same adapter, driver and wgpu version: the byte-equality rule applies.
    pub fn same_renderer(&self, other: &Manifest) -> bool {
        (&self.adapter, &self.driver, &self.wgpu) == (&other.adapter, &other.driver, &other.wgpu)
    }

    /// Whether the picture is a zebra image.
    pub fn is_zebra(&self) -> bool {
        self.look == "zebra"
    }
}

/// A picture and its manifest.
#[derive(Clone, Debug, PartialEq)]
pub struct Stored {
    pub image: Image,
    pub manifest: Manifest,
}

fn io(path: &Path, e: impl std::fmt::Display) -> String {
    format!("{}: {e}", path.display())
}

fn pair(dir: &Path, name: &str) -> (PathBuf, PathBuf) {
    (
        dir.join(format!("{name}.png")),
        dir.join(format!("{name}.toml")),
    )
}

/// The picture called `name` in `dir`, if there is one.
pub fn read(dir: &Path, name: &str) -> Result<Option<Stored>, String> {
    let (png, toml_path) = pair(dir, name);
    if !png.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&png).map_err(|e| io(&png, e))?;
    let image = Image::from_png(&bytes).map_err(|e| io(&png, e))?;
    let text = std::fs::read_to_string(&toml_path).map_err(|e| io(&toml_path, e))?;
    let manifest = toml::from_str(&text).map_err(|e| io(&toml_path, e))?;
    Ok(Some(Stored { image, manifest }))
}

/// Writes a picture as `name` in `dir`, replacing one of that name.
pub fn write(dir: &Path, name: &str, picture: &Stored) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
    let (png, toml_path) = pair(dir, name);
    let bytes = picture.image.png().map_err(|e| io(&png, e))?;
    std::fs::write(&png, bytes).map_err(|e| io(&png, e))?;
    let text = toml::to_string(&picture.manifest).map_err(|e| io(&toml_path, e))?;
    std::fs::write(&toml_path, text).map_err(|e| io(&toml_path, e))
}

fn remove(dir: &Path, name: &str) -> Result<(), String> {
    let (png, toml_path) = pair(dir, name);
    for path in [png, toml_path] {
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| io(&path, e))?;
        }
    }
    Ok(())
}

/// The names of the pictures in `dir`, sorted; none if it does not exist.
pub fn names(dir: &Path) -> Result<Vec<String>, String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(Vec::new());
    };
    let mut out: Vec<String> = (entries.filter_map(Result::ok))
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "png"))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    out.sort();
    Ok(out)
}

/// How one picture of a check came out.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// It passes against its reference.
    Matches(Verdict),
    /// It fails against its reference; it is now in the pending queue.
    Changed(Verdict),
    /// It has no reference; it is now in the pending queue.
    New,
}

/// One picture of a check.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub name: String,
    pub status: Status,
}

fn need(found: Option<Stored>, what: &str, name: &str) -> Result<Stored, String> {
    found.ok_or_else(|| format!("{name}: no {what} picture of that name"))
}

/// Removes the named pictures from `dir`; every one must be there.
fn forget(dir: &Path, what: &str, names: &[String]) -> Result<(), String> {
    for name in names {
        need(read(dir, name)?, what, name)?;
        remove(dir, name)?;
    }
    Ok(())
}

impl Goldens {
    /// The repository's folders: `goldens/`, `target/golden/` and `assets/matcaps/`.
    pub fn repo() -> Goldens {
        let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let top = crate_dir.ancestors().nth(2).unwrap_or(crate_dir);
        Goldens {
            root: top.join("goldens"),
            work: top.join("target/golden"),
            matcaps: top.join("assets/matcaps"),
        }
    }

    pub fn scenes(&self) -> PathBuf {
        self.root.join("scenes")
    }
    pub fn reference(&self) -> PathBuf {
        self.root.join("reference")
    }
    /// References an approval replaced, kept until the maintainer keeps or reverts them.
    pub fn previous(&self) -> PathBuf {
        self.root.join("previous")
    }
    pub fn pending(&self) -> PathBuf {
        self.work.join("pending")
    }
    /// `failures/<name>/` holds `expected.png`, `actual.png` and `diff.png` of a failed picture.
    pub fn failures(&self) -> PathBuf {
        self.work.join("failures")
    }

    /// Renders every scene on `r` and compares each picture with its reference by the rule
    /// in [`super::compare`]. A picture that does not pass goes to the pending queue, with its
    /// expected, actual and difference pictures in `failures/<name>/`. References are never
    /// written; only [`Goldens::approve`] changes them.
    pub fn check(&self, r: &Renderer) -> Result<Vec<Outcome>, String> {
        let mut out = Vec::new();
        for scene in GoldenScene::load_all(&self.scenes())? {
            let parts = scene.run()?;
            let defects = scene.defects(&parts);
            for look in &scene.looks {
                let name = scene.picture(look);
                let image = render(r, &parts, scene.how(), look, &self.matcaps)?;
                let manifest = Manifest::new(r.gpu(), &scene.name, &look.text(), defects.clone());
                let actual = Stored { image, manifest };
                out.push(self.judge(name, &actual)?);
            }
        }
        Ok(out)
    }

    /// Compares one picture with its reference and queues it unless it passes.
    fn judge(&self, name: String, actual: &Stored) -> Result<Outcome, String> {
        let reference = read(&self.reference(), &name)?;
        let status = match &reference {
            None => Status::New,
            Some(r) => {
                let same = r.manifest.same_renderer(&actual.manifest);
                let verdict = compare(&r.image, &actual.image, same, actual.manifest.is_zebra());
                if verdict.pass {
                    remove(&self.pending(), &name)?;
                    return Ok(Outcome {
                        name,
                        status: Status::Matches(verdict),
                    });
                }
                Status::Changed(verdict)
            }
        };
        write(&self.pending(), &name, actual)?;
        let dir = self.failures().join(&name);
        std::fs::create_dir_all(&dir).map_err(|e| io(&dir, e))?;
        let mut files = vec![("actual", actual.image.clone())];
        if let Some(r) = &reference {
            files.push(("expected", r.image.clone()));
            files.push(("diff", diff_map(&r.image, &actual.image)));
        }
        for (file, image) in files {
            let path = dir.join(format!("{file}.png"));
            let bytes = image.png().map_err(|e| io(&path, e))?;
            std::fs::write(&path, bytes).map_err(|e| io(&path, e))?;
        }
        Ok(Outcome { name, status })
    }

    /// Moves pending pictures over their references. The reference each replaces is kept in
    /// `previous/` for [`Goldens::revert`] and the digest, unless an earlier, unconfirmed
    /// approval already put one there (that older one is what the maintainer last saw).
    pub fn approve(&self, names: &[String]) -> Result<(), String> {
        for name in names {
            let new = need(read(&self.pending(), name)?, "pending", name)?;
            if let Some(old) = read(&self.reference(), name)?
                && read(&self.previous(), name)?.is_none()
            {
                write(&self.previous(), name, &old)?;
            }
            write(&self.reference(), name, &new)?;
            remove(&self.pending(), name)?;
        }
        Ok(())
    }

    /// Drops pending pictures.
    pub fn reject(&self, names: &[String]) -> Result<(), String> {
        forget(&self.pending(), "pending", names)
    }

    /// Puts back the reference an approval replaced. A brand-new reference had none, so it
    /// cannot be reverted; remove its scene instead.
    pub fn revert(&self, names: &[String]) -> Result<(), String> {
        for name in names {
            let old = need(read(&self.previous(), name)?, "replaced", name)?;
            write(&self.reference(), name, &old)?;
            remove(&self.previous(), name)?;
        }
        Ok(())
    }

    /// Confirms approvals: forgets the references they replaced.
    pub fn keep(&self, names: &[String]) -> Result<(), String> {
        forget(&self.previous(), "replaced", names)
    }

    /// The maintainer's digest: every reference an approval replaced and nobody has kept or
    /// reverted yet, old next to new, as one sheet written to `out`. Returns how many; with
    /// none, nothing is written.
    pub fn digest(&self, out: &Path) -> Result<usize, String> {
        let changed = names(&self.previous())?;
        if changed.is_empty() {
            return Ok(0);
        }
        sheet(&self.previous(), &self.reference(), &changed, out)
    }

    /// A report of the pending queue (each picture against its reference, and what the mesh
    /// checks found) and of approvals awaiting the maintainer, with a sheet of the pending
    /// pictures written to `work/review.png`.
    pub fn review(&self) -> Result<String, String> {
        let pending = names(&self.pending())?;
        let mut lines = vec![format!("pending ({}):", pending.len())];
        for name in &pending {
            let new = need(read(&self.pending(), name)?, "pending", name)?;
            let what = match read(&self.reference(), name)? {
                None => "new, no reference".to_string(),
                Some(old) => {
                    let same = old.manifest.same_renderer(&new.manifest);
                    compare(&old.image, &new.image, same, new.manifest.is_zebra()).summary()
                }
            };
            lines.push(format!("  {name}: {what}"));
            lines.extend(
                new.manifest
                    .defects
                    .iter()
                    .map(|d| format!("    mesh check: {d}")),
            );
        }
        if !pending.is_empty() {
            let out = self.work.join("review.png");
            sheet(&self.reference(), &self.pending(), &pending, &out)?;
            lines.push(format!("sheet: {}", out.display()));
        }
        let awaiting = names(&self.previous())?;
        lines.push(format!(
            "approved, awaiting the maintainer's look ({}): {}",
            awaiting.len(),
            awaiting.join(", ")
        ));
        Ok(lines.join("\n"))
    }
}

/// One sheet of every picture in `names`: what `before` holds next to what `after` holds,
/// and FLIP's difference map, three tiles per row. Pictures missing from `before` show
/// as background. Written as PNG to `out`; returns how many rows it has.
fn sheet(before: &Path, after: &Path, names: &[String], out: &Path) -> Result<usize, String> {
    let mut tiles: Vec<(String, Image)> = Vec::new();
    for name in names {
        let new = need(read(after, name)?, "current", name)?.image;
        let old = read(before, name)?.map_or_else(
            || Image::filled(new.width, new.height, gs_render::BACKGROUND),
            |s| s.image,
        );
        let diff = diff_map(&old, &new);
        tiles.push((format!("{name} before"), old));
        tiles.push(("after".into(), new));
        tiles.push(("difference".into(), diff));
    }
    let refs: Vec<(&str, &Image)> = tiles.iter().map(|(l, i)| (l.as_str(), i)).collect();
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
    }
    let png = Image::sheet(&refs, 3).png().map_err(|e| io(out, e))?;
    std::fs::write(out, png).map_err(|e| io(out, e))?;
    Ok(names.len())
}
