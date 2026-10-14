//! The safe replace (decision 0016, section 4): a failure at any one file operation leaves the
//! old file byte-identical and readable and no temporary file behind; a replace that runs to
//! the end keeps the old file as the backup.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gs_io::scene::{Disk, DiskFile, Kept, RealDisk, backup_path, read_bytes, replace};

use crate::sample::{file, store};

/// The real file system, except that operation number `fail` (counting from 0) fails.
#[derive(Clone)]
struct FailingDisk {
    fail: usize,
    done: Arc<AtomicUsize>,
}

impl FailingDisk {
    fn step(&self) -> io::Result<()> {
        if self.done.fetch_add(1, Ordering::SeqCst) == self.fail {
            return Err(io::Error::other("injected failure"));
        }
        Ok(())
    }
}

struct FailingFile {
    disk: FailingDisk,
    file: Box<dyn DiskFile>,
}

impl Write for FailingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.disk.step()?;
        self.file.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl DiskFile for FailingFile {
    fn sync(&mut self) -> io::Result<()> {
        self.disk.step()?;
        self.file.sync()
    }
}

/// Operations on two paths: each counts as one step, then runs on the real disk.
macro_rules! counted {
    ($($op:ident),*) => {$(
        fn $op(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.step().and_then(|()| RealDisk.$op(from, to))
        }
    )*};
}

impl Disk for FailingDisk {
    counted!(copy_permissions, hard_link, copy, rename);

    fn create_new(&self, path: &Path) -> io::Result<Box<dyn DiskFile>> {
        self.step()?;
        let file = RealDisk.create_new(path)?;
        let disk = self.clone();
        Ok(Box::new(FailingFile { disk, file }))
    }
    fn remove(&self, path: &Path) -> io::Result<()> {
        self.step().and_then(|()| RealDisk.remove(path))
    }
    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        self.step().and_then(|()| RealDisk.sync_dir(dir))
    }
    fn free(&self, dir: &Path) -> Option<u64> {
        RealDisk.free(dir)
    }
}

/// A fresh folder under the test runner's temporary folder, `scene-replace/<name>-<process>`.
fn folder(name: &str) -> PathBuf {
    let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("scene-replace");
    let dir = base.join(format!("{name}-{}", std::process::id()));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("an old folder");
    }
    std::fs::create_dir_all(&dir).expect("folder");
    dir
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("list")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_failure_at_any_step_leaves_the_old_file_as_it_was() {
    let dir = folder("faults");
    let target = dir.join("scene.gss");
    let backup = backup_path(&target);
    let old = file(&[store(6, 1)], None);
    let new = file(&[store(8, 2)], None);
    let save = |fail: usize| {
        std::fs::write(&target, &old).expect("old file");
        std::fs::remove_file(&backup).ok();
        let disk = FailingDisk {
            fail,
            done: Arc::default(),
        };
        let result = replace(&disk, &target, Some(&backup), |out| out.write_all(&new));
        (result, disk.done.load(Ordering::SeqCst))
    };
    let (clean, steps) = save(usize::MAX);
    clean.expect("a replace with no failure succeeds");
    assert_eq!(
        std::fs::read(&backup).expect("backup"),
        old,
        "the backup is the old file"
    );
    assert_eq!(names(&dir), ["scene.backup.gss", "scene.gss"]);
    assert!(
        steps >= 8,
        "create, permissions, write, sync, backup, link, rename, folder: {steps}"
    );
    for k in 0..steps {
        let (result, _) = save(k);
        let now = std::fs::read(&target).expect("the target is there");
        assert!(
            now == old || now == new,
            "step {k}: the target is a whole file"
        );
        assert!(
            read_bytes(&now, Kept::None).is_ok(),
            "step {k}: and it reads"
        );
        if result.is_ok() {
            assert!(
                now == new,
                "step {k}: a replace that succeeded wrote the new file"
            );
        } else if now == old {
            let left = names(&dir);
            assert!(
                left.iter()
                    .all(|n| Path::new(n).extension().is_none_or(|e| e != "tmp")),
                "step {k}: temporary file left: {left:?}"
            );
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_replace_without_backup_keeps_none_and_a_new_file_needs_none() {
    let dir = folder("plain");
    let target = dir.join("new.gss");
    let bytes = file(&[store(4, 3)], None);
    replace(&RealDisk, &target, Some(&backup_path(&target)), |out| {
        out.write_all(&bytes)
    })
    .expect("first save");
    assert_eq!(
        names(&dir),
        ["new.gss"],
        "nothing to back up the first time"
    );
    replace(&RealDisk, &target, None, |out| out.write_all(&bytes)).expect("second save");
    assert_eq!(names(&dir), ["new.gss"], "no backup when asked for none");
    std::fs::remove_dir_all(&dir).ok();
}
