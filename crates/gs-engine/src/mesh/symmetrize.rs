//! `mesh.symmetrize` (decision 0021, section 10): cut a part at a mirror plane, keep one side,
//! mirror it and weld the cut, so the part is exactly symmetric and carries its mirror map.

use std::collections::BTreeMap;

use gs_core::{AttrDef, AttrKind, AttrValue, Domain};
use gs_protocol::{
    AttrDomain, AttrValues, Attribute, CommandError, Kind, NO_VERTEX, PartRef, PlainMesh,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::corners;
use crate::command::PartArg;
use crate::document::pinned;
use crate::level::uv_corners;
use crate::sculpt::Axis;
use crate::{Command, Ctx, Example, Unavailable, errors, register_command};

/// Make a part exactly symmetric across a plane through the origin: keep one side, mirror it
/// onto the other and weld the two along the plane.
///
/// Faces crossing the plane are cut at it; points within a millionth of the part's size of the
/// plane move onto it; faces lying in the plane go. Every loop where the surface crosses the
/// plane becomes a weld, so a surface crossing it in more than one loop (a dent poking through,
/// say) comes out with a tunnel for each extra loop. Point values are blended along cuts by
/// their kind (mask and colors blend, a label is taken from the nearer end) and copied to the
/// mirrored side; face values are copied, and UVs stay on their face corners (blended along a
/// cut, reversed with a mirrored face's winding). The result carries the mirror map across the
/// plane, so mirrored Live Detail strokes keep its topology exactly symmetric (`symmetry.map`).
/// A part with detail levels is refused.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshSymmetrize {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// The axis square to the plane: `x` mirrors across the plane `x = 0`.
    pub axis: Axis,
    /// The side kept: `positive` (the default) keeps the points with a coordinate of 0 or more
    /// along the axis.
    #[serde(default)]
    pub side: Side,
}

/// Which side of the plane a symmetrize keeps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    #[default]
    Positive,
    Negative,
}

/// The symmetric part.
#[derive(Serialize, JsonSchema)]
pub struct Symmetrized {
    pub part: String,
    pub vertices: u64,
    pub faces: u64,
    /// Points on the plane, each its own mirror partner.
    pub on_plane: u64,
}

/// Points this close to the plane, as a fraction of the part's bounding-box diagonal, move
/// onto it.
const SNAP: f32 = 1e-6;

impl Command for MeshSymmetrize {
    const NAME: &'static str = "mesh.symmetrize";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    const CLAIMS: &'static str = r#"[{"local": {"part": "/part"}}]"#;
    type Output = Symmetrized;

    fn examples() -> Vec<Example<Self>> {
        let keep = |axis, side| Self {
            part: None,
            axis,
            side,
        };
        vec![
            Example::new(
                "mirror a sculpted sphere's +X half",
                keep(Axis::X, Side::Positive),
            )
            .after("primitive.sphere", json!({ "subdivisions": 4 }))
            .after("mesh.jitter", json!({ "amount": "2%", "seed": 3 })),
            Example::new("keep a cube's lower half", keep(Axis::Y, Side::Negative))
                .after("primitive.cube", json!({})),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Symmetrized, CommandError> {
        let part = pinned(self.part)?;
        cx.doc.single_level(&part, "symmetrize")?;
        // In the part's own space, where its symmetry planes are (decision 0029, section 7).
        let mesh = cx.doc.local(&part)?;
        let snap = SNAP * cx.doc.part_diagonal(&part)?;
        let out = symmetrize(&mesh, self.axis as usize, self.side, snap).ok_or_else(|| {
            errors::unavailable(
                Unavailable::new(format!("no face of part '{part}' lies on the kept side"))
                    .fix("keep the other side"),
            )
        })?;
        let reply = Symmetrized {
            part: part.clone(),
            vertices: out.mesh.positions.len() as u64,
            faces: out.mesh.faces.len() as u64,
            on_plane: out.on_plane as u64,
        };
        cx.doc.insert_rebuilt(&part, out.mesh, "symmetrize")?;
        Ok(reply)
    }
}
register_command!(MeshSymmetrize, "Mirror and weld");

/// A symmetrized mesh and how many of its points lie on the plane.
pub(crate) struct Mirrored {
    pub mesh: PlainMesh,
    pub on_plane: usize,
}

/// A point of the kept half: an old point, or a cut between two (`t` of the way from the
/// first to the second).
#[derive(Clone, Copy)]
enum Half {
    Old(u32),
    Cut(u32, u32, f32),
}

/// Where a face corner of the kept half comes from on its old face: corner `a`, or `t` of the
/// way from corner `a` to corner `b` where an edge was cut.
#[derive(Clone, Copy, Default)]
struct Corner {
    a: usize,
    b: usize,
    t: f32,
}

/// A face of the symmetric mesh: its points, the old face it came from, and where each of its
/// corners comes from on that face.
#[derive(Clone, Copy)]
struct Kept {
    face: [u32; 4],
    old: usize,
    corners: [Corner; 4],
}

/// `mesh` cut at the plane square to `axis`, the `side` half kept, mirrored and welded, with
/// the map column; `None` if no face is left. Points within `snap` of the plane move onto it.
pub(crate) fn symmetrize(mesh: &PlainMesh, axis: usize, side: Side, snap: f32) -> Option<Mirrored> {
    let sign = if side == Side::Positive { 1.0 } else { -1.0 };
    let mut pos = mesh.positions.clone();
    for p in &mut pos {
        if p[axis].abs() <= snap {
            p[axis] = 0.0;
        }
    }
    let d: Vec<f32> = pos.iter().map(|p| sign * p[axis]).collect();
    let (half, faces) = keep(mesh, &d);
    if faces.is_empty() {
        return None;
    }
    let n = half.len();
    let mut positions: Vec<[f32; 3]> = half.iter().map(|&h| place(&pos, h, axis)).collect();
    // Each point off the plane gets a mirrored copy after the half's points.
    let mut partner: Vec<u32> = (0..n as u32).collect();
    for i in 0..n {
        if positions[i][axis] != 0.0 {
            let mut q = positions[i];
            q[axis] = -q[axis];
            partner[i] = positions.len() as u32;
            partner.push(i as u32);
            positions.push(q);
        }
    }
    let on_plane = (0..n).filter(|&i| partner[i] == i as u32).count();
    // A mirrored face's winding is reversed, and its corners with it.
    let mirrored = faces.iter().map(|f| {
        let k = corners(&f.face).len();
        let mut g = Kept {
            face: [NO_VERTEX; 4],
            ..*f
        };
        for i in 0..k {
            g.face[k - 1 - i] = partner[f.face[i] as usize];
            g.corners[k - 1 - i] = f.corners[i];
        }
        g
    });
    let all: Vec<Kept> = faces.iter().copied().chain(mirrored).collect();
    let uv = uv_corners();
    let mut attributes = BTreeMap::new();
    for (name, a) in &mesh.attributes {
        if gs_core::mirror::is_map(name) {
            continue;
        }
        let values = match a.domain {
            AttrDomain::Vertex => point_values(name, a, &half, &partner, &d),
            AttrDomain::Face => match corner_of(&uv, name) {
                Some((group, k)) => corner_values(mesh, a, group, k, &all),
                None => column(&a.values, all.len(), |f| value(&a.values, all[f].old)),
            },
        };
        attributes.insert(
            name.clone(),
            Attribute {
                domain: a.domain,
                values,
            },
        );
    }
    attributes.insert(
        gs_core::mirror::def(axis).name,
        Attribute {
            domain: AttrDomain::Vertex,
            values: AttrValues::U32(partner),
        },
    );
    let mesh = PlainMesh {
        positions,
        faces: all.into_iter().map(|x| x.face).collect(),
        attributes,
    };
    Some(Mirrored { mesh, on_plane })
}

/// The kept half of `mesh` (where `d`, the signed distance to the plane, is above 0, plus the
/// plane): its points, each once (an old point by its index, a cut by its edge), and its faces.
fn keep(mesh: &PlainMesh, d: &[f32]) -> (Vec<Half>, Vec<Kept>) {
    let mut half: Vec<Half> = Vec::new();
    let mut slots: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    let mut take = |h: Half| -> u32 {
        let key = match h {
            Half::Old(i) => (i, NO_VERTEX),
            Half::Cut(i, j, _) => (i.min(j), i.max(j)),
        };
        *slots.entry(key).or_insert_with(|| {
            half.push(h);
            half.len() as u32 - 1
        })
    };
    let mut faces: Vec<Kept> = Vec::new();
    for (old, fc) in mesh.faces.iter().enumerate() {
        let c = corners(fc);
        let ds: Vec<f32> = c.iter().map(|&v| d[v as usize]).collect();
        if ds.iter().all(|&x| x <= 0.0) {
            continue;
        }
        let mut poly: Vec<(u32, Corner)> = Vec::new();
        for k in 0..c.len() {
            let next = (k + 1) % c.len();
            let (i, j) = (c[k], c[next]);
            let (di, dj) = (ds[k], ds[next]);
            if di >= 0.0 {
                let at = Corner { a: k, b: k, t: 0.0 };
                poly.push((take(Half::Old(i)), at));
            }
            if di * dj < 0.0 {
                let t = di / (di - dj);
                poly.push((take(Half::Cut(i, j, t)), Corner { a: k, b: next, t }));
            }
        }
        // A triangle or quad stays one face; a cut quad's pentagon becomes a fan of triangles.
        let face = |of: &[(u32, Corner)]| {
            let mut kept = Kept {
                face: [NO_VERTEX; 4],
                old,
                corners: [Corner::default(); 4],
            };
            for (i, &(v, at)) in of.iter().enumerate() {
                kept.face[i] = v;
                kept.corners[i] = at;
            }
            kept
        };
        if poly.len() <= 4 {
            faces.push(face(&poly));
        } else {
            for k in 1..poly.len() - 1 {
                faces.push(face(&[poly[0], poly[k], poly[k + 1]]));
            }
        }
    }
    (half, faces)
}

/// Where a point of the half sits; a cut lies exactly on the plane.
fn place(pos: &[[f32; 3]], h: Half, axis: usize) -> [f32; 3] {
    match h {
        Half::Old(i) => pos[i as usize],
        Half::Cut(i, j, t) => {
            let (a, b) = (pos[i as usize], pos[j as usize]);
            let mut p = [0.0; 3];
            for k in 0..3 {
                p[k] = a[k] + (b[k] - a[k]) * t;
            }
            p[axis] = 0.0;
            p
        }
    }
}

/// The definition a store built from the result gives column `name`, so it blends as it will
/// be kept (gs-core's import rule; a column the import refuses blends by its values' type).
fn def(name: &str, a: &Attribute) -> AttrDef {
    AttrDef::of_plain(name, a).unwrap_or_else(|_| {
        let point = a.domain == AttrDomain::Vertex;
        let domain = if point { Domain::Vertex } else { Domain::Face };
        let kind = match a.values {
            AttrValues::F32(_) => AttrKind::Scalar,
            AttrValues::U32(_) => AttrKind::Label,
        };
        AttrDef::new(name, domain, kind)
    })
}

/// Value `i` of a column.
fn value(v: &AttrValues, i: usize) -> AttrValue {
    match v {
        AttrValues::F32(x) => AttrValue::F32(x[i]),
        AttrValues::U32(x) => AttrValue::U32(x[i]),
    }
}

/// A column stored like `like`, of `len` values from `at`.
fn column(like: &AttrValues, len: usize, at: impl Fn(usize) -> AttrValue) -> AttrValues {
    let bits = |x: AttrValue| match x {
        AttrValue::F32(x) => x.to_bits(),
        AttrValue::U32(x) => x,
    };
    match like {
        AttrValues::F32(_) => {
            AttrValues::F32((0..len).map(|i| f32::from_bits(bits(at(i)))).collect())
        }
        AttrValues::U32(_) => AttrValues::U32((0..len).map(|i| bits(at(i))).collect()),
    }
}

/// A point column over the symmetric mesh: the half's values (blended along a cut as the
/// column keeps them, decision 0037, with the kept end first so a label's tie goes to it), then
/// each mirrored copy's from its original.
fn point_values(
    name: &str,
    a: &Attribute,
    half: &[Half],
    partner: &[u32],
    d: &[f32],
) -> AttrValues {
    let (def, v, n) = (def(name, a), &a.values, half.len());
    column(v, partner.len(), |i| {
        match half[if i < n { i } else { partner[i] as usize }] {
            Half::Old(k) => value(v, k as usize),
            Half::Cut(i, j, t) => {
                let (x, y) = (value(v, i as usize), value(v, j as usize));
                if d[i as usize] > 0.0 {
                    AttrValue::blend_weighted(&def, &[x, y], &[1.0 - t, t])
                } else {
                    AttrValue::blend_weighted(&def, &[y, x], &[t, 1.0 - t])
                }
            }
        }
    })
}

/// The corner group and corner of column `name`, if it is one of the UV corners.
fn corner_of<'a>(groups: &'a [[String; 4]], name: &str) -> Option<(&'a [String; 4], usize)> {
    (groups.iter()).find_map(|g| Some((g, g.iter().position(|c| c == name)?)))
}

/// Corner `k` of a UV corner column over the symmetric mesh: each face corner's value read from
/// its old face's corner, blended along the edge where it was cut; a triangle's corner 3 is 0.
fn corner_values(
    mesh: &PlainMesh,
    a: &Attribute,
    group: &[String; 4],
    k: usize,
    faces: &[Kept],
) -> AttrValues {
    let def = def(&group[k], a);
    let of = |c: usize| mesh.attributes.get(&group[c]).map(|col| &col.values);
    column(&a.values, faces.len(), |f| {
        let (face, at) = (&faces[f], faces[f].corners[k]);
        let read = |c: usize| of(c).map_or(AttrValue::zero(def.kind), |v| value(v, face.old));
        if k >= corners(&face.face).len() {
            AttrValue::zero(def.kind)
        } else if at.a == at.b {
            read(at.a)
        } else {
            AttrValue::blend_weighted(&def, &[read(at.a), read(at.b)], &[1.0 - at.t, at.t])
        }
    })
}
