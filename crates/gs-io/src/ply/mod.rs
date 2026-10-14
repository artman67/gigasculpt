//! Stanford PLY, ASCII and binary (either byte order), following the public format
//! description.
//!
//! Read: the `vertex` element's `x`, `y`, `z` and, when present, `red`, `green`, `blue` and
//! `alpha` (bytes, or numbers from 0 to 1), and the `face` element's `vertex_indices` (or
//! `vertex_index`) list, polygons split (see [`crate::mesh`]). Every other property and element
//! is read past and counted in the notes: normals, UVs, other scalars. One object, unnamed.
//!
//! Write: positions as `float`, colors as four `uchar`s when asked and any object has them
//! (white for the others), faces as a `uchar` count and `int` indices. Several objects go into
//! one vertex and one face list. Binary little endian, or ASCII with each `f32` as the shortest
//! decimal text that reads back to it.

mod data;
mod header;
mod write;

use std::io::{BufReader, Read};

use crate::ReadError;
use crate::mesh::{Builder, Lines, MeshFile, Notes, ReadOptions, words};

use data::{Binary, Source, Words, walk};
use header::{Element, Format, Header, Kind};

pub use write::write;

/// The most corners one polygon may have.
const MAX_CORNERS: f64 = 65536.0;

/// Reads a PLY file. See the module docs for what is kept.
pub fn read(source: impl Read, options: &ReadOptions) -> Result<MeshFile, ReadError> {
    let mut lines = Lines::new(BufReader::with_capacity(1 << 16, source));
    let header = header::read(&mut lines)?;
    let mut sink = Sink::new(&header).map_err(|reason| ReadError::BadLine {
        line: lines.line,
        reason,
    })?;
    match header.format {
        Format::Ascii => {
            lines.each(|line, text| sink.line(line, text))?;
            if let Some(k) = sink.pending() {
                return Err(ReadError::BadLine {
                    line: lines.line + 1,
                    reason: format!(
                        "the file ends inside the {} element",
                        header.elements[k].name
                    ),
                });
            }
        }
        Format::Binary { big_endian } => {
            let offset = lines.offset;
            let mut src = Binary {
                r: lines.into_inner(),
                big_endian,
                offset,
            };
            sink.binary(&mut src)?;
        }
    }
    let (b, notes) = (sink.b, sink.notes);
    let objects = if b.face_count() > 0 {
        vec![b.finish(None, options.group_polygons)]
    } else {
        Vec::new()
    };
    Ok(MeshFile { objects, notes })
}

/// What an element is for.
enum Role {
    Vertex(VertexLayout),
    /// Faces, with the index of their corner list property.
    Face(usize),
    Skip,
}

/// Where a vertex element keeps the properties we use.
struct VertexLayout {
    xyz: [usize; 3],
    rgba: [Option<usize>; 4],
    /// Whether the color properties hold bytes (else numbers from 0 to 1).
    bytes: bool,
}

/// The properties of a vertex we use.
const VERTEX_PROPS: [&str; 7] = ["x", "y", "z", "red", "green", "blue", "alpha"];

/// Takes the element instances in file order into the object being built.
struct Sink<'h> {
    header: &'h Header,
    roles: Vec<Role>,
    b: Builder,
    notes: Notes,
    /// The element being read and how many of its instances are read.
    at: (usize, u64),
    row: Vec<f64>,
    list: Vec<f64>,
    ids: Vec<u32>,
}

impl<'h> Sink<'h> {
    fn new(header: &'h Header) -> Result<Self, String> {
        let mut notes = Notes::default();
        let (mut vertices, mut faces) = (false, false);
        let mut roles = Vec::new();
        for el in &header.elements {
            let role = match el.name.as_str() {
                "vertex" if !vertices => {
                    vertices = true;
                    vertex_role(el, &mut notes)?
                }
                "face" if !faces => {
                    faces = true;
                    face_role(el, &mut notes)?
                }
                _ => {
                    notes.ignore(format!("element {}", el.name), el.count);
                    Role::Skip
                }
            };
            roles.push(role);
        }
        let mut sink = Self {
            header,
            roles,
            b: Builder::new(String::new()),
            notes,
            at: (0, 0),
            row: Vec::new(),
            list: Vec::new(),
            ids: Vec::new(),
        };
        sink.skip_empty();
        Ok(sink)
    }

    /// The element whose instances are still to come, if any.
    fn pending(&self) -> Option<usize> {
        (self.at.0 < self.header.elements.len()).then_some(self.at.0)
    }

    /// Moves past elements with nothing (left) to read.
    fn skip_empty(&mut self) {
        let els = &self.header.elements;
        for _ in 0..els.len() {
            match els.get(self.at.0) {
                Some(el) if self.at.1 == el.count || el.props.is_empty() => {
                    self.at = (self.at.0 + 1, 0);
                }
                _ => return,
            }
        }
    }

    /// Takes one ASCII line as the next instance; `false` once every element is read.
    fn line(&mut self, line: u64, text: &[u8]) -> Result<bool, ReadError> {
        let Some(k) = self.pending() else {
            return Ok(false);
        };
        if text.trim_ascii().is_empty() {
            return Ok(true);
        }
        let mut src = Words {
            words: words(text),
            line,
        };
        self.instance(k, &mut src)?;
        if src.words.next().is_some() {
            return Err(src.bad("more values than the element has properties".into()));
        }
        self.at.1 += 1;
        self.skip_empty();
        Ok(self.pending().is_some())
    }

    /// Reads every instance of every element from binary data.
    fn binary(&mut self, src: &mut Binary<impl Read>) -> Result<(), ReadError> {
        let header = self.header;
        for (k, el) in header.elements.iter().enumerate() {
            if el.props.is_empty() {
                continue;
            }
            for _ in 0..el.count {
                self.instance(k, src)?;
            }
        }
        Ok(())
    }

    /// Reads one instance of element `k` and takes it in.
    fn instance(&mut self, k: usize, src: &mut impl Source) -> Result<(), ReadError> {
        let el = &self.header.elements[k];
        let list_at = match self.roles[k] {
            Role::Face(at) => Some(at),
            _ => None,
        };
        walk(el, &mut self.row, list_at, &mut self.list, src)?;
        self.take(k).map_err(|reason| src.bad(reason))
    }

    fn take(&mut self, k: usize) -> Result<(), String> {
        let row = &self.row;
        match &self.roles[k] {
            Role::Vertex(layout) => {
                let v = self.b.positions.len();
                let p = layout.xyz.map(|i| row[i] as f32);
                if p.iter().any(|x| !x.is_finite()) {
                    return Err("a vertex position is not a finite number".into());
                }
                self.b.positions.push(p);
                if layout.rgba.iter().flatten().any(|&i| !row[i].is_finite()) {
                    return Err("a vertex color is not a finite number".into());
                }
                if layout.rgba[..3].iter().all(Option::is_some) {
                    let channel = |c: Option<usize>| match c {
                        Some(i) if layout.bytes => row[i].clamp(0.0, 255.0) as u8,
                        Some(i) => (row[i].clamp(0.0, 1.0) * 255.0).round() as u8,
                        None => 255,
                    };
                    self.b.paint_vertex(
                        v,
                        Some(u32::from_le_bytes(layout.rgba.map(channel))),
                        None,
                    );
                }
            }
            Role::Face(_) => {
                let nv = self.b.positions.len() as f64;
                self.ids.clear();
                for &c in &self.list {
                    if !(c >= 0.0 && c < nv && c.fract() == 0.0) {
                        return Err(format!("a face refers to vertex {c}, but there are {nv}"));
                    }
                    self.ids.push(c as u32);
                }
                if self.ids.len() < 3 {
                    return Err("a face needs at least 3 corners".into());
                }
                self.b.add_polygon(&self.ids, None, None, &mut self.notes);
            }
            Role::Skip => {}
        }
        Ok(())
    }
}

fn vertex_role(el: &Element, notes: &mut Notes) -> Result<Role, String> {
    let find = |name: &str| {
        el.props
            .iter()
            .position(|p| p.name == name && p.kind.is_scalar())
    };
    let at = |name| find(name).ok_or_else(|| format!("a vertex needs a scalar {name}"));
    for p in &el.props {
        if !VERTEX_PROPS.contains(&p.name.as_str()) {
            notes.ignore(format!("vertex property {}", p.name), el.count);
        }
    }
    if el.count >= u64::from(u32::MAX) {
        return Err(format!(
            "{} vertices are more than a mesh can hold",
            el.count
        ));
    }
    Ok(Role::Vertex(VertexLayout {
        xyz: [at("x")?, at("y")?, at("z")?],
        rgba: ["red", "green", "blue", "alpha"].map(find),
        bytes: find("red").is_some_and(|i| !el.props[i].kind.is_float()),
    }))
}

fn face_role(el: &Element, notes: &mut Notes) -> Result<Role, String> {
    let list = el.props.iter().position(|p| {
        let named = ["vertex_indices", "vertex_index"].contains(&p.name.as_str());
        named && matches!(p.kind, Kind::List(..))
    });
    let list = list.ok_or("a face needs a vertex_indices list")?;
    for (i, p) in el.props.iter().enumerate() {
        if i != list {
            notes.ignore(format!("face property {}", p.name), el.count);
        }
    }
    Ok(Role::Face(list))
}
