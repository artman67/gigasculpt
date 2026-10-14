//! Free space (decision 0016, section 17, item 9): what a folder the session writes to has free,
//! read through a seam a test can set instead of filling a disk, and the low disk warning
//! `session.state` gives as `file.low_disk` with every call.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gs_io::scene::{Disk, RealDisk};
use schemars::JsonSchema;
use serde::Serialize;

use crate::document::Document;

/// The warning shows under this much free space, however small the scene: ten autosave copies
/// of a 20 million triangle scene.
const FLOOR: u64 = 1 << 30;

/// A folder the session writes to with less free space than a save and the next one need.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct LowDisk {
    /// The scene file's folder, or the state folder autosave writes in.
    pub folder: String,
    /// Its free bytes.
    pub free: u64,
    /// The bytes it warns under: the larger of 1 GiB and twice the scene file's estimate.
    pub threshold: u64,
}

/// The first of the scene file's folder (`file`'s) and the state folder (`state`) with less
/// free space than the larger of 1 GiB and twice what `doc`'s file would take, by the estimate
/// a save's own check makes (a third of the columns' raw size: 12 bytes a point, 16 a face).
/// `None` while both have room, or neither is known.
pub(crate) fn low_disk(
    free: &FreeSpace,
    file: Option<&Path>,
    state: Option<&Path>,
    doc: &Document,
) -> Option<LowDisk> {
    let folder = file
        .and_then(Path::parent)
        .filter(|p| !p.as_os_str().is_empty());
    let folders = [folder, state];
    let raw: u64 = (doc.parts())
        .map(|(_, p)| (p.store().vertex_count() * 12 + p.store().face_count() * 16) as u64)
        .sum();
    let threshold = FLOOR.max(raw / 3 * 2);
    folders.into_iter().flatten().find_map(|f| {
        let left = free.of(f)?;
        (left < threshold).then(|| LowDisk {
            folder: f.display().to_string(),
            free: left,
            threshold,
        })
    })
}

/// Where free space comes from: the file system (the default), or a fixed amount a test sets
/// (`EngineConfig::free_space`).
#[derive(Clone, Debug, Default)]
pub struct FreeSpace(Option<Arc<AtomicU64>>);

impl FreeSpace {
    /// `bytes` free in every folder, until [`FreeSpace::set`] changes it.
    #[must_use]
    pub fn fixed(bytes: u64) -> Self {
        Self(Some(Arc::new(AtomicU64::new(bytes))))
    }

    /// Changes a fixed amount, for every clone of it; the file system's cannot be set.
    pub fn set(&self, bytes: u64) {
        if let Some(fixed) = &self.0 {
            fixed.store(bytes, Ordering::Relaxed);
        }
    }

    /// The bytes free in `folder`, if known.
    pub fn of(&self, folder: &Path) -> Option<u64> {
        match &self.0 {
            Some(fixed) => Some(fixed.load(Ordering::Relaxed)),
            None => RealDisk.free(folder),
        }
    }
}
