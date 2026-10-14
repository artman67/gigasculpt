//! Send to Blender and Read from Blender (decision 0025, section 13): two controls running
//! `exchange.write` and `exchange.read` (decision 0032) with the exchange folder given by its
//! name, `to-blender` or `from-blender`. The window turns the name into the session's folder
//! exactly as the Blender add-on finds it, and makes `to-blender` before a send as the add-on
//! does. GigaSculpt cannot tell whether Blender is running (the add-on connects only for each of
//! its own operations), so the buttons show the folders' state: Read is held back, "Nothing new
//! from Blender", unless `from-blender` holds a manifest written since the window first looked
//! and since the last `exchange.read` any client ran, so the same objects are never read twice.

use std::path::PathBuf;
use std::time::SystemTime;

use gs_protocol::Step;
use serde_json::Value;

/// The folder names a control may give, each the session's folder of that name.
const NAMES: [&str; 2] = ["to-blender", "from-blender"];

/// Why Read from Blender is held back.
const NOTHING_NEW: &str = "Nothing new from Blender";

/// The cache folder the add-on uses: `$XDG_CACHE_HOME`, or `~/.cache` when the variable is
/// unset or not absolute. `None` with neither.
fn cache() -> Option<PathBuf> {
    let set = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from);
    let home = || std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache"));
    set.filter(|p| p.is_absolute()).or_else(home)
}

/// The exchange folders, and what the window has counted as read from `from-blender`.
pub struct Exchange {
    /// The cache folder they are under.
    cache: Option<PathBuf>,
    /// Whether the window has looked at `from-blender` yet.
    looked: bool,
    /// When the manifest counted as read was written; `None` when there was none.
    seen: Option<SystemTime>,
}

impl Default for Exchange {
    /// The folders under the cache folder the add-on uses.
    fn default() -> Self {
        Self {
            cache: cache(),
            looked: false,
            seen: None,
        }
    }
}

impl Exchange {
    /// The folders under `cache` instead (a test's own folder).
    pub fn at(cache: PathBuf) -> Self {
        Self {
            cache: Some(cache),
            ..Self::default()
        }
    }

    /// The session's exchange folder `name`: `<cache>/gigasculpt/exchange/<session>/<name>`, as
    /// the add-on finds it. `None` before the window knows its session.
    pub fn folder(&self, session: &str, name: &str) -> Option<PathBuf> {
        let cache = self.cache.as_ref().filter(|_| !session.is_empty())?;
        Some(cache.join("gigasculpt/exchange").join(session).join(name))
    }

    /// Gives the exchange commands in `steps` that name their folder by name the session's
    /// folder of that name, making `to-blender` first when `make` (a send about to run).
    pub fn resolve(&self, steps: &mut [Step], session: &str, make: bool) {
        for step in steps
            .iter_mut()
            .filter(|s| s.command.starts_with("exchange."))
        {
            let named = step.args.get("folder").and_then(Value::as_str);
            let Some(name) = named.filter(|n| NAMES.contains(n)) else {
                continue;
            };
            let Some(path) = self.folder(session, name) else {
                continue;
            };
            if make && name == "to-blender" {
                std::fs::create_dir_all(&path).ok();
            }
            step.args["folder"] = Value::from(path.to_string_lossy().into_owned());
        }
    }

    /// When `session`'s `from-blender` manifest was written, if there is one.
    fn written(&self, session: &str) -> Option<SystemTime> {
        let manifest = self.folder(session, "from-blender")?.join("manifest.json");
        std::fs::metadata(manifest).and_then(|m| m.modified()).ok()
    }

    /// Counts what `from-blender` holds now as read: an `exchange.read` ran.
    pub fn read(&mut self, session: &str) {
        self.seen = self.written(session);
        self.looked = true;
    }

    /// Why Read from Blender would bring nothing new, if it would not. What the folder holds
    /// when the window first looks counts as read.
    pub fn nothing_new(&mut self, session: &str) -> Option<String> {
        if session.is_empty() {
            return Some(NOTHING_NEW.into());
        }
        if !self.looked {
            self.read(session);
        }
        (self.written(session) <= self.seen).then(|| NOTHING_NEW.into())
    }
}
