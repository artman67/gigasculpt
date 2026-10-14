//! Replacing a file without ever risking the last good one (decision 0016, section 4;
//! research 15, section 3.1): write a hidden temporary file beside the target, flush it to
//! disk, keep the old file as the backup through a hard link, rename the new file over the
//! target, then flush the folder (on Unix: see [`RealDisk`]'s `sync_dir`). Until the rename the
//! target is untouched; any failure before it removes the temporary file.
//!
//! Every operation goes through [`Disk`], so tests can make each one fail in turn.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A file being written.
pub trait DiskFile: Write + Send {
    /// Flushes the file's bytes to the disk.
    fn sync(&mut self) -> io::Result<()>;
}

impl DiskFile for File {
    fn sync(&mut self) -> io::Result<()> {
        self.sync_all()
    }
}

/// The file operations a replace uses.
pub trait Disk: Sync {
    /// Creates `path`, failing if it exists.
    fn create_new(&self, path: &Path) -> io::Result<Box<dyn DiskFile>>;
    /// Gives `to` the permissions of `from`.
    fn copy_permissions(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn remove(&self, path: &Path) -> io::Result<()>;
    fn hard_link(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn copy(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// Flushes a folder's entries to the disk.
    fn sync_dir(&self, dir: &Path) -> io::Result<()>;
    /// Bytes free for this user in the folder `dir`, if the system says.
    fn free(&self, dir: &Path) -> Option<u64>;
}

/// The real file system.
pub struct RealDisk;

impl Disk for RealDisk {
    fn create_new(&self, path: &Path) -> io::Result<Box<dyn DiskFile>> {
        let file = OpenOptions::new().write(true).create_new(true).open(path)?;
        Ok(Box::new(file))
    }
    fn copy_permissions(&self, from: &Path, to: &Path) -> io::Result<()> {
        fs::set_permissions(to, fs::metadata(from)?.permissions())
    }
    fn remove(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }
    fn hard_link(&self, from: &Path, to: &Path) -> io::Result<()> {
        fs::hard_link(from, to)
    }
    fn copy(&self, from: &Path, to: &Path) -> io::Result<()> {
        fs::copy(from, to).map(drop)
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }
    #[cfg(unix)]
    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        File::open(dir)?.sync_all()
    }
    /// Skipped: Windows cannot open a folder as a file, and there the rename itself is what
    /// makes the new file appear whole.
    #[cfg(not(unix))]
    fn sync_dir(&self, _: &Path) -> io::Result<()> {
        Ok(())
    }
    #[cfg(unix)]
    fn free(&self, dir: &Path) -> Option<u64> {
        let s = rustix::fs::statvfs(dir).ok()?;
        Some(s.f_bavail.saturating_mul(s.f_frsize))
    }
    #[cfg(not(unix))]
    fn free(&self, _: &Path) -> Option<u64> {
        None
    }
}

/// The backup name of a scene file: `name.gss` keeps its previous version as
/// `name.backup.gss` (decision 0016, section 4).
pub fn backup_path(target: &Path) -> PathBuf {
    let stem = target.file_stem().unwrap_or_default().to_string_lossy();
    let ext = target.extension().map(|e| e.to_string_lossy().into_owned());
    let name = match ext {
        Some(ext) => format!("{stem}.backup.{ext}"),
        None => format!("{stem}.backup"),
    };
    target.with_file_name(name)
}

/// A hidden temporary name beside `target`, unique in the process.
fn temporary(target: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    target.with_file_name(format!(".{name}.{}.{n}.tmp", std::process::id()))
}

/// The folder a path lives in.
pub(crate) fn folder(path: &Path) -> &Path {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    }
}

/// Writes `target` through `write`, safely (see the module docs), keeping the old file as
/// `backup` when given and the target exists.
pub fn replace(
    disk: &dyn Disk,
    target: &Path,
    backup: Option<&Path>,
    write: impl FnOnce(&mut dyn Write) -> io::Result<()>,
) -> io::Result<()> {
    let temp = temporary(target);
    let file = disk.create_new(&temp)?;
    let done = fill(disk, target, &temp, backup, file, write);
    if done.is_err() {
        disk.remove(&temp).ok();
        return done;
    }
    disk.sync_dir(folder(target))
}

/// Every step up to and including the rename.
fn fill(
    disk: &dyn Disk,
    target: &Path,
    temp: &Path,
    backup: Option<&Path>,
    file: Box<dyn DiskFile>,
    write: impl FnOnce(&mut dyn Write) -> io::Result<()>,
) -> io::Result<()> {
    let old = fs::symlink_metadata(target).is_ok();
    if old {
        disk.copy_permissions(target, temp)?;
    }
    let mut out = BufWriter::with_capacity(1 << 20, file);
    write(&mut out)?;
    let mut file = out.into_inner().map_err(io::IntoInnerError::into_error)?;
    file.sync()?;
    drop(file);
    if let (true, Some(backup)) = (old, backup) {
        match disk.remove(backup) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        disk.hard_link(target, backup)
            .or_else(|_| disk.copy(target, backup))?;
    }
    disk.rename(temp, target)
}
