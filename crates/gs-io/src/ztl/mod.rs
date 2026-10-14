//! Reader for ZTL tool files (.ZTL) and ZPR project files (.ZPR), learned only from files
//! (clean room: never from the program that writes them). Layout notes, totals, guesses and
//! open questions: `docs/decisions/0005-ztl-format.md`.
//!
//! A tool file is the shared 92-byte preamble, an icon and one tool object. A project file has
//! the same preamble and icon, then a project object whose sections hold the canvas, the camera
//! and every loaded tool. A tool holds its parts; each part has a name, the tool palette
//! settings it was saved with, a list entry (flags, color, display name) and its geometry: the
//! base faces, the positions of the level it was at and of its top level, per vertex mask and
//! color, per face groups, flags and UVs.
//!
//! [`open`] reads the header and the list of tools and parts without decoding any geometry;
//! [`ZtlFile::geometry`] then decodes one part at a time, reading only that part's bytes, so a
//! 2 GB file never sits in memory whole. [`read`] does both for a file already in memory.

mod geometry;
mod list;
mod project;

use std::io::{Read, Seek};

pub use crate::container::{
    Curve, CurvePoint, Image, NamedImage, Preamble, RawObject, Setting, SettingKind,
};
pub use geometry::{ArrayInfo, Export, Geometry, Level, MeshData, Slot};
/// The subdivision numbering tool files use; its one home is gs-core (decision 0006).
pub use gs_core::subdivide_faces;

use crate::ReadError;
use crate::container::chunk::{Cursor, Source, Span, Stream, object_at};
use crate::container::{PREAMBLE, picture};

/// The preamble tags of a tool file and a project file.
const TAGS: [&[u8; 4]; 2] = [b"LTZ\0", b"RPZ\0"];
/// The version of a tool object.
const TOOL_VERSION: u16 = 1004;

/// Whether a file is a tool or a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Tool,
    Project,
}

/// What every tool and project file starts with.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub kind: FileKind,
    pub preamble: Preamble,
    pub icon: NamedImage,
    /// Project files: the project object and its sections.
    pub project: Option<Project>,
}

/// The project object of a project file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub version: u16,
    /// Every section (canvas, camera, timeline, tools and more) with the tag stored before it;
    /// the tools section has tag 2.
    pub sections: Vec<(u32, RawSection)>,
}

/// An object kept by location, so a reader can come back for it: where its start marker is,
/// its version and its total length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawSection {
    pub offset: usize,
    pub version: u16,
    pub len: usize,
}

impl RawSection {
    fn of(span: Span) -> Self {
        Self {
            offset: span.offset,
            version: span.version,
            len: span.len,
        }
    }
}

/// One tool: its parts in list order.
#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    /// File offset of the tool object.
    pub offset: usize,
    /// Project files: the byte stored before the tool (1, or 2 for one tool per project).
    pub project_flag: Option<u8>,
    /// The tool object's leading plain fields: a fixed label, text not understood yet and the
    /// selected part's type number.
    pub head: Vec<u8>,
    /// The parts in the order the part list shows them.
    pub parts: Vec<PartInfo>,
    /// Index into `parts` of the part that was selected when the file was saved. Its record
    /// holds the rest of the list.
    pub selected: usize,
}

/// What the list says about one part; its geometry comes from [`ZtlFile::geometry`].
#[derive(Debug, Clone, PartialEq)]
pub struct PartInfo {
    /// The name stored with the settings: usually the part's name, sometimes an import path.
    pub name: String,
    /// The type number stored before the name: 11 for a polygon mesh, 0 for a sphere armature.
    pub kind: u32,
    /// The byte stored before the type number (0 for polygon meshes, 1 for sphere armatures).
    pub lead: u8,
    /// The part list entry.
    pub entry: Entry,
    /// The entry the part's own record keeps for itself (seen on parts that were added from
    /// another tool); `None` for the selected part and most others.
    pub own_entry: Option<Entry>,
    /// More mesh objects stored after the part's own (seen once: a sphere armature's chain
    /// next to its preview mesh), kept by location.
    pub extra: Vec<RawSection>,
    /// The tool palette settings saved with the part.
    pub settings: Settings,
    /// Version of the part's record object (10, 11 or 13 seen).
    pub record_version: u16,
    /// The record's leading plain fields; for the selected part they hold its index and the
    /// part count.
    pub record_head: Vec<u8>,
    mesh: Span,
    companion: Option<Span>,
}

/// One entry of the part list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Flags not understood yet (0x10, 0x11, 0x21 and more seen).
    pub flags: u32,
    /// Four bytes that look like a color (white for most parts).
    pub color: [u8; 4],
    /// The name the list shows.
    pub name: String,
    /// A second name, empty in most entries; probably the folder the part is in (a guess: it
    /// is set exactly when flag 0x800 is, and the names look like folder names).
    pub folder: Option<String>,
    /// The fixed-length fields after the names, not understood yet.
    pub rest: Vec<u8>,
}

/// The tool palette settings saved with a part: plain fields, then records like a brush's
/// settings, each after a presence byte.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// The plain fields before the first record, its presence byte included.
    pub head: Vec<u8>,
    pub records: Vec<Setting>,
}

/// A tool or project file opened for reading on demand.
pub struct ZtlFile<R> {
    pub header: Header,
    pub tools: Vec<Tool>,
    src: Stream<R>,
}

impl<R: Read + Seek> ZtlFile<R> {
    /// Decodes the geometry of one part, reading only its bytes.
    pub fn geometry(&mut self, tool: usize, part: usize) -> Result<Geometry, ReadError> {
        let info =
            self.tools
                .get(tool)
                .and_then(|t| t.parts.get(part))
                .ok_or(ReadError::BadLayout {
                    offset: 0,
                    reason: "no such tool or part",
                })?;
        geometry::decode(&mut self.src, info.mesh, info.companion)
    }
}

/// A whole tool or project file, every part decoded.
#[derive(Debug, Clone)]
pub struct Ztl {
    pub header: Header,
    pub tools: Vec<Tool>,
    /// `geometry[t][s]` belongs to `tools[t].parts[s]`.
    pub geometry: Vec<Vec<Geometry>>,
}

/// Opens a tool or project file: reads the header and every tool's part list.
pub fn open<R: Read + Seek>(source: R) -> Result<ZtlFile<R>, ReadError> {
    let mut src = Stream::new(source)?;
    let (header, tools) = read_index(&mut src)?;
    Ok(ZtlFile { header, tools, src })
}

/// Reads a whole tool or project file in memory, decoding every part. For big files use
/// [`open`] and decode one part at a time.
pub fn read(bytes: &[u8]) -> Result<Ztl, ReadError> {
    let mut file = open(std::io::Cursor::new(bytes))?;
    let counts: Vec<usize> = file.tools.iter().map(|t| t.parts.len()).collect();
    let mut geometry = Vec::new();
    for (t, &count) in counts.iter().enumerate() {
        let parts = (0..count).map(|s| file.geometry(t, s));
        geometry.push(parts.collect::<Result<_, _>>()?);
    }
    Ok(Ztl {
        header: file.header,
        tools: file.tools,
        geometry,
    })
}

/// The header and the tools, read from the front of the file.
fn read_index<R: Read + Seek>(src: &mut Stream<R>) -> Result<(Header, Vec<Tool>), ReadError> {
    let front = src.bytes(0, src.end().min(PREAMBLE), "preamble")?;
    let (preamble, kind) = match crate::container::preamble(&front, &TAGS)? {
        Some((p, 0)) => (p, FileKind::Tool),
        Some((p, _)) => (p, FileKind::Project),
        None => return Err(ReadError::NotZtl),
    };
    let (icon, after_icon) = icon(src)?;
    let main = object_at(src, after_icon, src.end(), "tool or project object")?;
    let (project, tools) = match kind {
        FileKind::Tool if main.version != TOOL_VERSION => {
            return Err(ReadError::BadLayout {
                offset: main.offset,
                reason: "a tool file holds a tool object after the icon",
            });
        }
        FileKind::Tool => (None, vec![list::tool(src, main, None)?]),
        FileKind::Project => {
            let (project, tools) = project::read(src, main)?;
            (Some(project), tools)
        }
    };
    let header = Header {
        kind,
        preamble,
        icon,
        project,
    };
    Ok((header, tools))
}

/// The icon after the preamble (name, rectangle and image container) and where it ends.
fn icon<R: Read + Seek>(src: &mut Stream<R>) -> Result<(NamedImage, usize), ReadError> {
    let mut len = 0;
    for _ in 0..3 {
        len += object_at(src, PREAMBLE + len, src.end(), "icon")?.len;
    }
    let bytes = src.bytes(PREAMBLE, len, "icon")?;
    let mut c = Cursor::new(&bytes, PREAMBLE);
    Ok((picture(&mut c, "icon")?, PREAMBLE + len))
}
