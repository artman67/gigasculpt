//! The palette (decision 0043, sections 3 and 5): the session's materials in order, the active
//! one, and the palette's own undo history, which the scene's undo never sees.

use std::sync::Arc;

use gs_core::material::{DEFAULT_LOOK, Library, Recipe, Refused};
use gs_protocol::{CommandError, ErrorCode, Role};

use crate::ctx::Origin;
use crate::errors::{args_invalid, error, not_found};

/// One material of the palette.
#[derive(Clone, Debug, PartialEq)]
pub struct PaletteEntry {
    /// Its id (`#n`): handed out once and never again.
    pub id: u64,
    /// What commands take, for example `red-wax`; unique in the palette.
    pub name: String,
    /// Shipped with the program; a copy (`material.copy`) is a user material.
    pub built_in: bool,
    /// The look: its material, environment and bake; its `name` is the title the window shows.
    pub recipe: Arc<Recipe>,
}

/// The session's materials in palette order and the active one: what the renderer draws every
/// look from (`RenderHost::materials`). Not the command palette (`ui.palette`).
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialPalette {
    pub materials: Vec<PaletteEntry>,
    /// The active material's id: what MatCap and Lit shading draw on every part.
    pub active: u64,
    /// Recipe files that were not read: none while only the shipped looks are read.
    pub refused: Vec<Refused>,
}

/// The built-ins shipped before decision 0043's step (c): they keep `#1` to `#13` and `@0` to
/// `@12`, and user materials follow them from `#14` and `@13`.
const FIRST: usize = 13;
/// The ids of built-ins shipped later, which sit after the user materials; no user material
/// takes one, so a log written before them names the same materials (step (c)).
pub(super) const LATER: std::ops::RangeInclusive<u64> = 1001..=1999;

/// The id of the look at `place` in the shipped list.
fn shipped_id(place: usize) -> u64 {
    match place.checked_sub(FIRST) {
        None => place as u64 + 1,
        Some(later) => LATER.start() + later as u64,
    }
}

impl MaterialPalette {
    /// The shipped looks in the shipped order, with MatCap Gray active: what every session and
    /// every renderer starts with. The first thirteen are `#1` to `#13`, the later ones `#1001`
    /// on.
    pub fn shipped() -> MaterialPalette {
        let library = Library::shipped();
        let materials: Vec<PaletteEntry> = (library.looks.into_iter())
            .enumerate()
            .map(|(place, look)| PaletteEntry {
                id: shipped_id(place),
                name: look.name,
                built_in: true,
                recipe: Arc::new(look.recipe),
            })
            .collect();
        let active = (materials.iter().find(|m| m.name == DEFAULT_LOOK)).map_or(1, |m| m.id);
        MaterialPalette {
            materials,
            active,
            refused: library.refused,
        }
    }

    /// The active material; the first one if the active id is gone.
    pub fn active(&self) -> &PaletteEntry {
        let active = self.materials.iter().find(|m| m.id == self.active);
        active.unwrap_or(&self.materials[0])
    }

    /// The material called `name`.
    pub fn get(&self, name: &str) -> Option<&PaletteEntry> {
        self.materials.iter().find(|m| m.name == name)
    }

    /// Where a new user material goes: before the built-ins shipped later (step (c)).
    pub(crate) fn user_end(&self) -> usize {
        let later = |m: &PaletteEntry| m.built_in && LATER.contains(&m.id);
        let at = self.materials.iter().position(later);
        at.unwrap_or(self.materials.len())
    }

    /// The material `key` names: `#id`, `@place` (from 0) or a name.
    pub(crate) fn find(&self, key: &str) -> Result<&PaletteEntry, CommandError> {
        let key = key.trim();
        let number = |s: &str| s.parse::<u64>().ok();
        let found = if let Some(id) = key.strip_prefix('#').and_then(number) {
            self.materials.iter().find(|m| m.id == id)
        } else if let Some(at) = key.strip_prefix('@').and_then(number) {
            usize::try_from(at)
                .ok()
                .and_then(|at| self.materials.get(at))
        } else {
            self.get(key)
        };
        found.ok_or_else(|| CommandError {
            hint: Some("material.list lists every material".into()),
            ..not_found(
                "material",
                key,
                self.materials.iter().map(|m| m.name.as_str()),
            )
        })
    }

    /// Refuses `name` for the material `id` (`None`: a new one): empty, starting with `#` or
    /// `@`, `main`, or another material's.
    pub(crate) fn check_name(&self, name: &str, id: Option<u64>) -> Result<(), CommandError> {
        let bad = |why: &str| {
            let message = format!("a material cannot be named '{name}': {why}");
            Err(args_invalid("/name", message))
        };
        if name.trim().is_empty() || name.trim() != name {
            return bad("a name is not empty and has no spaces at its ends");
        }
        if name.starts_with(['#', '@']) {
            return bad("# and @ start ids and places");
        }
        // The main view's name, which a Reference View's look takes for the main view's look.
        if name == crate::view::MAIN {
            return bad("a Reference View's look uses main for the main view's");
        }
        match self.get(name) {
            Some(other) if Some(other.id) != id => Err(CommandError {
                hint: Some("material.rename the other one, or choose another name".into()),
                ..error(
                    ErrorCode::Precondition,
                    format!("material #{} is already named '{name}'", other.id),
                )
            }),
            _ => Ok(()),
        }
    }
}

/// Palette steps kept for its Undo, like each view's camera history.
const HISTORY: usize = 50;

/// One step of the palette's history: the materials as they were on the other side of it.
#[derive(Clone, Debug)]
struct Step {
    materials: Vec<PaletteEntry>,
    command: &'static str,
    /// Who made it: client name and role.
    author: (String, Role),
}

/// The palette and its own history: session state, in no scene step (decision 0043, section 5).
#[derive(Clone, Debug)]
pub(crate) struct PaletteState {
    pub now: MaterialPalette,
    /// Ids handed out so far; the next is one more. Never goes back, so undo never reissues one.
    pub(super) issued: u64,
    back: Vec<Step>,
    ahead: Vec<Step>,
    /// Palette steps made, undone and redone: the scene has unsaved changes while this differs
    /// from the count its file was saved or opened at (decision 0043, section 7).
    pub steps: u64,
    /// The user folders `material.folder` read this session, as given: an open keeps them, and
    /// reads them again into a file's place when it has no palette (section 7).
    pub folders: Vec<String>,
    /// The shipped palette an open landed in place of a file's (none, or one that does not
    /// read), the folders not yet read into it.
    pub bare: bool,
}

impl Default for PaletteState {
    fn default() -> Self {
        PaletteState {
            now: MaterialPalette::shipped(),
            issued: FIRST as u64,
            back: Vec::new(),
            ahead: Vec::new(),
            steps: 0,
            folders: Vec::new(),
            bare: false,
        }
    }
}

/// A step undone or redone.
pub(crate) struct Moved {
    pub command: &'static str,
    pub client: String,
}

impl PaletteState {
    /// The palette without its history: a job's copy, since a job runs no palette command.
    pub fn without_history(&self) -> PaletteState {
        PaletteState {
            now: self.now.clone(),
            issued: self.issued,
            back: Vec::new(),
            ahead: Vec::new(),
            steps: self.steps,
            folders: self.folders.clone(),
            bare: false,
        }
    }

    /// Takes an opened file's palette in place of this one, keeping the folders read.
    pub fn open(&mut self, mut opened: PaletteState) {
        opened.folders = std::mem::take(&mut self.folders);
        *self = opened;
    }

    /// Forgets every step to undo or redo: after the user folder is read (`material.folder`),
    /// so the first Undo never takes its materials away.
    pub fn forget(&mut self) {
        self.back.clear();
        self.ahead.clear();
    }

    /// A new material id: never one of the later built-ins' block.
    pub fn next_id(&mut self) -> u64 {
        self.issued += 1;
        if LATER.contains(&self.issued) {
            self.issued = LATER.end() + 1;
        }
        self.issued
    }

    /// The palette with `materials` instead, as one step of its history made by `command` for
    /// `origin`; clears what could be redone.
    pub fn record(&mut self, materials: Vec<PaletteEntry>, command: &'static str, origin: &Origin) {
        if self.back.len() == HISTORY {
            self.back.remove(0);
        }
        self.back.push(Step {
            materials: std::mem::replace(&mut self.now.materials, materials),
            command,
            author: (origin.client.clone(), origin.role),
        });
        self.ahead.clear();
        self.steps += 1;
        self.keep_active();
    }

    /// Whether there is a step to undo (`forward` false) or redo.
    pub fn can_travel(&self, forward: bool) -> bool {
        !if forward { &self.ahead } else { &self.back }.is_empty()
    }

    /// Undoes (`forward` false) or redoes the next step, if `origin` may: a human any step,
    /// anyone else only steps made under their own client name and role.
    pub fn travel(&mut self, forward: bool, origin: &Origin) -> Result<Moved, CommandError> {
        let which = if forward {
            "step to redo"
        } else {
            "step to undo"
        };
        let (from, to) = if forward {
            (&mut self.ahead, &mut self.back)
        } else {
            (&mut self.back, &mut self.ahead)
        };
        let Some(step) = from.last() else {
            return Err(error(
                ErrorCode::Precondition,
                format!("the palette has no {which}"),
            ));
        };
        let theirs =
            origin.role != Role::Human && step.author != (origin.client.clone(), origin.role);
        if theirs {
            return Err(CommandError {
                hint: Some("a palette step is undone by whoever made it, or by the human".into()),
                ..error(
                    ErrorCode::Stale,
                    format!(
                        "the {which} is {} by {}, not one of yours",
                        step.command, step.author.0
                    ),
                )
            });
        }
        let Some(mut step) = from.pop() else {
            return Err(error(
                ErrorCode::Precondition,
                format!("the palette has no {which}"),
            ));
        };
        std::mem::swap(&mut step.materials, &mut self.now.materials);
        let moved = Moved {
            command: step.command,
            client: step.author.0.clone(),
        };
        to.push(step);
        self.steps += 1;
        self.keep_active();
        Ok(moved)
    }

    /// Makes the first material active when the active one is gone.
    fn keep_active(&mut self) {
        let active = self.now.active().id;
        self.now.active = active;
    }
}
