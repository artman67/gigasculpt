//! Reading OBJ.

use std::collections::BTreeMap;
use std::io::{BufReader, Read};

use crate::ReadError;
use crate::mesh::{Builder, Lines, MeshFile, Notes, ReadOptions, finite, words};

/// The group faces belong to before any `g` line.
const DEFAULT_GROUP: &str = "default";

/// Reads an OBJ file. See the module docs for what is kept.
pub fn read(source: impl Read, options: &ReadOptions) -> Result<MeshFile, ReadError> {
    let mut lines = Lines::new(BufReader::with_capacity(1 << 16, source));
    let mut r = Reader::new(*options);
    lines.each(|n, line| {
        let read = r.line(line);
        read.map(|()| true)
            .map_err(|reason| ReadError::BadLine { line: n, reason })
    })?;
    Ok(r.finish())
}

/// One object block: the vertices defined in it and the faces after its `o` line.
struct Block {
    b: Builder,
    /// Global index of the first vertex defined in the block.
    first: u32,
    /// The local index of each vertex defined in the block, once a face has borrowed a vertex
    /// of another block (until then the two are the same).
    own: Option<Vec<u32>>,
    /// Vertices of other blocks the block's faces use: global index to local.
    borrowed: BTreeMap<u32, u32>,
    /// Group names to ids, in order of first use.
    groups: BTreeMap<String, u32>,
    named_group: bool,
    /// The block's next vertex a `#MRGB` entry is for.
    mrgb: u32,
}

impl Block {
    fn new(name: String, first: u32) -> Self {
        Self {
            b: Builder::new(name),
            first,
            own: None,
            borrowed: BTreeMap::new(),
            groups: BTreeMap::new(),
            named_group: false,
            mrgb: 0,
        }
    }

    /// The local index of global vertex `v`, defined in this block.
    fn own_local(&self, v: u32) -> usize {
        let i = (v - self.first) as usize;
        self.own.as_ref().map_or(i, |own| own[i] as usize)
    }
}

struct Reader {
    options: ReadOptions,
    done: Vec<Block>,
    cur: Block,
    /// Vertices defined so far, in every block.
    vertices: u32,
    uvs: Vec<[f32; 2]>,
    group: String,
    material: String,
    notes: Notes,
    /// Scratch for one face.
    corners: Vec<u32>,
    corner_uvs: Vec<[f32; 2]>,
}

type Line<T> = Result<T, String>;

impl Reader {
    fn new(options: ReadOptions) -> Self {
        Self {
            options,
            done: Vec::new(),
            cur: Block::new(String::new(), 0),
            vertices: 0,
            uvs: Vec::new(),
            group: DEFAULT_GROUP.into(),
            material: DEFAULT_GROUP.into(),
            notes: Notes::default(),
            corners: Vec::new(),
            corner_uvs: Vec::new(),
        }
    }

    fn line(&mut self, line: &[u8]) -> Line<()> {
        let mut w = words(line);
        let Some(key) = w.next() else {
            return Ok(());
        };
        let ignored = match key {
            b"v" => return self.vertex(w),
            b"vt" => {
                let uv = numbers::<3>(w).filter(|n| n.1 >= 2);
                let (uv, _) = uv.ok_or("a UV needs 2 or 3 finite numbers")?;
                self.uvs.push([uv[0], uv[1]]);
                return Ok(());
            }
            b"f" => return self.face(w),
            b"o" => {
                let next = Block::new(rest(line, key), self.vertices);
                let block = std::mem::replace(&mut self.cur, next);
                self.done.push(block);
                return Ok(());
            }
            b"g" => {
                self.group = rest(line, key);
                return Ok(());
            }
            b"usemtl" => {
                self.material = rest(line, key);
                return Ok(());
            }
            b"#MRGB" => return self.mrgb(w.next().unwrap_or_default()),
            b"vn" => "normals",
            b"mtllib" => "material libraries",
            b"s" => "smoothing groups",
            _ if key.starts_with(b"#") => return Ok(()),
            _ => "other lines",
        };
        self.notes.ignore(ignored, 1);
        Ok(())
    }

    /// `v x y z`, with an optional weight (`v x y z w`) or color (`v x y z r g b`, and
    /// `v x y z w r g b`).
    fn vertex<'a>(&mut self, w: impl Iterator<Item = &'a [u8]>) -> Line<()> {
        let bad = "a vertex needs 3 finite numbers, 4 with a weight, 6 or 7 with a color";
        let (n, count) = numbers::<7>(w).ok_or(bad)?;
        let rgb = match count {
            3 | 4 => None,
            6 | 7 => Some(&n[count - 3..count]),
            _ => return Err(bad.into()),
        };
        let more = u32::try_from(u64::from(self.vertices) + 1);
        self.vertices = more.map_err(|_| "more vertices than a mesh can hold")?;
        let v = self.cur.b.positions.len();
        self.cur.b.positions.push([n[0], n[1], n[2]]);
        if let Some(own) = &mut self.cur.own {
            own.push(v as u32);
        }
        if let Some(rgb) = rgb {
            let byte = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
            let c = u32::from_le_bytes([byte(rgb[0]), byte(rgb[1]), byte(rgb[2]), 255]);
            self.cur.b.paint_vertex(v, Some(c), None);
        }
        Ok(())
    }

    /// `#MRGB` followed by 8 hex digits (mask, red, green, blue) per vertex, for the vertices
    /// of the object block in order (a file without `o` lines is one block).
    fn mrgb(&mut self, hex: &[u8]) -> Line<()> {
        let (entries, rest) = hex.as_chunks::<8>();
        if !rest.is_empty() {
            return Err("an #MRGB line holds 8 hex digits per vertex".into());
        }
        for entry in entries {
            let mut b = [0u8; 4];
            for (byte, pair) in b.iter_mut().zip(entry.as_chunks::<2>().0) {
                let text = std::str::from_utf8(pair).ok();
                *byte = text
                    .and_then(|t| u8::from_str_radix(t, 16).ok())
                    .ok_or("an #MRGB line holds a character that is not a hex digit")?;
            }
            let v = self.cur.mrgb;
            self.cur.mrgb += 1;
            if v >= self.vertices - self.cur.first {
                self.notes.ignore("#MRGB entries past the last vertex", 1);
                continue;
            }
            let local = self.cur.own_local(self.cur.first + v);
            // Unmasked and white are what a vertex without an entry gets anyway.
            let mask = (b[0] != 0xFF).then(|| 1.0 - f32::from(b[0]) / 255.0);
            let color = (b[1..] != [0xFF; 3]).then(|| u32::from_le_bytes([b[1], b[2], b[3], 255]));
            self.cur.b.paint_vertex(local, color, mask);
        }
        Ok(())
    }

    /// The finished block that defined global vertex `v` (below the current block's first).
    fn block_of(&self, v: u32) -> &Block {
        // Blocks are in file order; of those starting at or before `v`, the last holds it
        // (an empty block before it starts at the same place).
        let k = self.done.partition_point(|d| d.first <= v) - 1;
        &self.done[k]
    }

    /// `f v`, `f v/vt`, `f v//vn` or `f v/vt/vn` for each corner; indices count from 1, or
    /// back from the last one read when negative.
    fn face<'a>(&mut self, w: impl Iterator<Item = &'a [u8]>) -> Line<()> {
        self.corners.clear();
        self.corner_uvs.clear();
        let mut with_uv = false;
        for word in w {
            let mut parts = word.split(|&b| b == b'/');
            let v = index(parts.next().unwrap_or_default(), self.vertices, "vertex")?;
            let local = self.local(v);
            self.corners.push(local);
            let uv = match parts.next().filter(|t| !t.is_empty()) {
                Some(t) => {
                    with_uv = true;
                    self.uvs[index(t, self.uvs.len() as u32, "UV")? as usize]
                }
                None => [0.0; 2],
            };
            self.corner_uvs.push(uv);
        }
        if self.corners.len() < 3 {
            return Err("a face needs at least 3 corners".into());
        }
        let name = if self.options.materials_as_groups {
            &self.material
        } else {
            &self.group
        };
        let next = self.cur.groups.len() as u32;
        let group = *self.cur.groups.entry(name.clone()).or_insert(next);
        self.cur.named_group |= name != DEFAULT_GROUP;
        let uvs = with_uv.then_some(&self.corner_uvs[..]);
        let notes = &mut self.notes;
        self.cur
            .b
            .add_polygon(&self.corners, uvs, Some(group), notes);
        Ok(())
    }

    /// The current block's index for global vertex `v`: its own, or a copy of a vertex of
    /// another block, made on first use.
    fn local(&mut self, v: u32) -> u32 {
        if v >= self.cur.first {
            return self.cur.own_local(v) as u32;
        }
        if let Some(&l) = self.cur.borrowed.get(&v) {
            return l;
        }
        let from = self.block_of(v);
        let at = from.own_local(v);
        let (p, (color, mask)) = (from.b.positions[at], from.b.paint(at));
        let cur = &mut self.cur;
        let l = cur.b.positions.len();
        if cur.own.is_none() {
            cur.own = Some((0..l as u32).collect());
        }
        cur.b.positions.push(p);
        cur.b.paint_vertex(l, color, mask);
        cur.borrowed.insert(v, l as u32);
        l as u32
    }

    fn finish(mut self) -> MeshFile {
        let last = std::mem::replace(&mut self.cur, Block::new(String::new(), 0));
        self.done.push(last);
        let mut objects = Vec::new();
        for block in self.done {
            let mut b = block.b;
            if b.face_count() == 0 {
                let points = b.positions.len() as u64;
                if points > 0 {
                    self.notes
                        .ignore("vertices of objects without faces", points);
                }
                continue;
            }
            let names = block.named_group.then(|| {
                let (ids, names) = group_ids(block.groups);
                b.renumber_groups(&ids);
                names
            });
            objects.push(b.finish(names, self.options.group_polygons));
        }
        MeshFile {
            objects,
            notes: self.notes,
        }
    }
}

/// The ids of groups numbered in order of first use, and the names by id: the number in every
/// name when all are `Group<n>` with distinct `n` (the form sculpting apps and our writer use,
/// so ids survive a round trip), else the order of first use.
fn group_ids(groups: BTreeMap<String, u32>) -> (Vec<u32>, BTreeMap<u32, String>) {
    let numbered: Option<Vec<(u32, u32)>> = (groups.iter())
        .map(|(name, &first)| {
            let n: u32 = name.strip_prefix("Group")?.parse().ok()?;
            (format!("Group{n}") == *name).then_some((first, n))
        })
        .collect();
    let mut ids: Vec<u32> = (0..groups.len() as u32).collect();
    for (first, n) in numbered.into_iter().flatten() {
        ids[first as usize] = n;
    }
    let names = (groups.into_iter())
        .map(|(name, first)| (ids[first as usize], name))
        .collect();
    (ids, names)
}

/// The finite numbers of a line's remaining words, at most `N`, and how many there were;
/// `None` if a word is not a finite number or there are more than `N`.
fn numbers<'a, const N: usize>(w: impl Iterator<Item = &'a [u8]>) -> Option<([f32; N], usize)> {
    let mut out = [0.0; N];
    let mut count = 0;
    for word in w {
        *out.get_mut(count)? = finite(word)?;
        count += 1;
    }
    Some((out, count))
}

/// A 1-based OBJ index (negative counts back from `count`), as a 0-based index below `count`.
fn index(word: &[u8], count: u32, what: &str) -> Line<u32> {
    let i: i64 = std::str::from_utf8(word)
        .ok()
        .and_then(|t| t.parse().ok())
        .ok_or_else(|| format!("a face corner's {what} index is not a whole number"))?;
    let at = match i.cmp(&0) {
        std::cmp::Ordering::Greater => i - 1,
        std::cmp::Ordering::Less => i64::from(count) + i,
        std::cmp::Ordering::Equal => -1,
    };
    if !(0..i64::from(count)).contains(&at) {
        return Err(format!(
            "a face refers to {what} {i}, but {count} are defined so far"
        ));
    }
    Ok(at as u32)
}

/// The rest of a line after its keyword, trimmed: a name.
fn rest(line: &[u8], key: &[u8]) -> String {
    let after = line.trim_ascii_start().get(key.len()..).unwrap_or_default();
    String::from_utf8_lossy(after.trim_ascii()).into_owned()
}
