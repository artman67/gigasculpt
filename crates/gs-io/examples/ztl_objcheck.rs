//! Checks decoded tool files against OBJ exports of the same models (not a gate; the files are
//! private and stay outside the repository).
//!
//! ```text
//! cargo run --release -p gs-io --example ztl_objcheck -- <pairs file> [report file]
//! ```
//!
//! Each line of the pairs file is a tool or project file and an OBJ export, separated by a tab.
//! An OBJ export holds the chosen level of one or more parts, one after another in list
//! order. The check finds which parts and stored levels add up to the OBJ's vertex and face
//! counts, then compares positions (after the export scale and offset each part stores),
//! face lists, face groups, UVs and the mask and color lines (`#MRGB`) when the OBJ has them.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Write as _};

use gs_io::ztl::{Geometry, open};
use gs_protocol::{AttrValues, NO_VERTEX};

/// What an OBJ export holds.
#[derive(Default)]
struct Obj {
    positions: Vec<[f64; 3]>,
    uvs: Vec<[f64; 2]>,
    /// Corners of each face: vertex index and UV index (0-based; `u32::MAX` for none).
    faces: Vec<Vec<(u32, u32)>>,
    /// The group number of each face, in the order group names first appear.
    groups: Vec<u32>,
    /// Mask and color per vertex: `M R G B`.
    mrgb: Vec<[u8; 4]>,
}

fn parse_obj(path: &str) -> std::io::Result<Obj> {
    let mut obj = Obj::default();
    let mut names: BTreeMap<String, u32> = BTreeMap::new();
    let mut group = 0;
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        let mut parts = line.split_ascii_whitespace();
        match parts.next() {
            Some("v") => {
                let v: Vec<f64> = parts.take(3).filter_map(|t| t.parse().ok()).collect();
                if let [x, y, z] = v[..] {
                    obj.positions.push([x, y, z]);
                }
            }
            Some("vt") => {
                let v: Vec<f64> = parts.take(2).filter_map(|t| t.parse().ok()).collect();
                if let [u, w] = v[..] {
                    obj.uvs.push([u, w]);
                }
            }
            Some("g") => {
                let name = parts.collect::<Vec<_>>().join(" ");
                let next = names.len() as u32;
                group = *names.entry(name).or_insert(next);
            }
            Some("f") => {
                let corners = parts
                    .map(|t| {
                        let mut ids = t.split('/').map(|i| i.parse::<u32>().ok());
                        let v = ids.next().flatten().map_or(u32::MAX, |i| i - 1);
                        let u = ids.next().flatten().map_or(u32::MAX, |i| i - 1);
                        (v, u)
                    })
                    .collect();
                obj.faces.push(corners);
                obj.groups.push(group);
            }
            Some("#MRGB") => {
                let hex = parts.next().unwrap_or_default().as_bytes();
                for chunk in hex.as_chunks::<8>().0 {
                    let byte = |i: usize| {
                        std::str::from_utf8(&chunk[i..i + 2])
                            .ok()
                            .and_then(|s| u8::from_str_radix(s, 16).ok())
                            .unwrap_or(0)
                    };
                    obj.mrgb.push([byte(0), byte(2), byte(4), byte(6)]);
                }
            }
            _ => {}
        }
    }
    Ok(obj)
}

/// One stored level of one part: its vertex and face counts.
struct Candidate {
    tool: usize,
    part: usize,
    level_index: usize,
    vertices: usize,
    faces: usize,
    /// The first few faces, to tell which order the export wrote the parts in.
    first: Vec<[u32; 4]>,
}

/// True if `c`'s first faces match the OBJ faces at `base`.
fn starts_at(c: &Candidate, obj: &Obj, base: (usize, usize)) -> bool {
    c.first.iter().enumerate().all(|(i, face)| {
        let theirs = obj
            .faces
            .get(base.1 + i)
            .map(|f| f.iter().map(|c| c.0 as usize));
        let ours = face
            .iter()
            .filter(|&&v| v != NO_VERTEX)
            .map(|&v| v as usize + base.0);
        theirs.is_some_and(|t| ours.eq(t))
    })
}

/// The order of `picked` in which every part's first faces match the export, trying list
/// order first.
fn order<'a>(picked: &[&'a Candidate], obj: &Obj) -> Vec<&'a Candidate> {
    fn fits(order: &[&Candidate], obj: &Obj) -> bool {
        let mut base = (0, 0);
        order.iter().all(|c| {
            let ok = starts_at(c, obj, base);
            base = (base.0 + c.vertices, base.1 + c.faces);
            ok
        })
    }
    fn permute(items: &mut Vec<&Candidate>, k: usize, obj: &Obj) -> bool {
        if k == items.len() {
            return fits(items, obj);
        }
        for i in k..items.len() {
            items.swap(k, i);
            if permute(items, k + 1, obj) {
                return true;
            }
            items.swap(k, i);
        }
        false
    }
    let mut items = picked.to_vec();
    if fits(&items, obj) || (items.len() <= 7 && permute(&mut items, 0, obj)) {
        return items;
    }
    picked.to_vec()
}

/// Picks, in list order, at most one stored level per part so the counts add up exactly;
/// prefers picks that skip fewer parts.
fn pick(cands: &[Vec<Candidate>], vertices: usize, faces: usize) -> Option<Vec<&Candidate>> {
    fn go<'a>(
        cands: &'a [Vec<Candidate>],
        at: usize,
        left: (usize, usize),
        chosen: &mut Vec<&'a Candidate>,
        best: &mut Option<Vec<&'a Candidate>>,
    ) {
        if left == (0, 0) {
            if best.as_ref().is_none_or(|b| chosen.len() > b.len()) {
                *best = Some(chosen.clone());
            }
            return;
        }
        let Some(options) = cands.get(at) else {
            return;
        };
        for c in options {
            if c.vertices <= left.0 && c.faces <= left.1 {
                chosen.push(c);
                go(
                    cands,
                    at + 1,
                    (left.0 - c.vertices, left.1 - c.faces),
                    chosen,
                    best,
                );
                chosen.pop();
            }
        }
        go(cands, at + 1, left, chosen, best);
    }
    let mut best = None;
    go(cands, 0, (vertices, faces), &mut Vec::new(), &mut best);
    best
}

#[derive(Default)]
struct Totals {
    pairs: u64,
    matched: u64,
    parts: u64,
    vertices: u64,
    faces: u64,
    faces_identical: u64,
    groups_checked: u64,
    groups_matching: u64,
    uv_corners: u64,
    uv_corners_matching: u64,
    mrgb_vertices: u64,
    mrgb_mask_matching: u64,
    mrgb_color_matching: u64,
    no_export: u64,
    worst_error: f64,
    worst_relative: f64,
    by_level: BTreeMap<u32, u64>,
}

/// Compares one part level against its stretch of the OBJ; returns a report line.
fn compare(
    g: &Geometry,
    level_index: usize,
    obj: &Obj,
    base: (usize, usize),
    totals: &mut Totals,
) -> String {
    let Some(m) = &g.mesh else {
        return "no polygon mesh".into();
    };
    let level = &m.levels[level_index];
    let mesh = &level.mesh;
    let (scale, offset) = m
        .export
        .map_or(([1.0; 3], [0.0; 3]), |e| (e.scale, e.offset));
    if m.export.is_none() {
        totals.no_export += 1;
    }
    let mut worst = 0f64;
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for (i, p) in mesh.positions.iter().enumerate() {
        let o = obj.positions[base.0 + i];
        for k in 0..3 {
            let expect = (f64::from(p[k]) - f64::from(offset[k])) / f64::from(scale[k]);
            worst = worst.max((expect - o[k]).abs());
            lo[k] = lo[k].min(o[k]);
            hi[k] = hi[k].max(o[k]);
        }
    }
    let size = (0..3).map(|k| hi[k] - lo[k]).fold(0f64, f64::max);
    let relative = if size > 0.0 { worst / size } else { 0.0 };
    let mut identical = 0;
    let mut shown = Vec::new();
    for (f, face) in mesh.faces.iter().enumerate() {
        let theirs = &obj.faces[base.1 + f];
        let ours = face
            .iter()
            .filter(|&&v| v != NO_VERTEX)
            .map(|&v| v as usize + base.0);
        if ours.eq(theirs.iter().map(|c| c.0 as usize)) {
            identical += 1;
        } else if shown.len() < 6 && std::env::var_os("SHOW_FACES").is_some() {
            let t: Vec<usize> = theirs.iter().map(|c| c.0 as usize - base.0).collect();
            shown.push(format!("face {f}: ours {face:?} theirs {t:?}"));
        }
    }
    for s in &shown {
        eprintln!("      {s}");
    }
    let line = format!(
        "level {} ({} of {}): {} vertices, {} faces; worst position error {:.3e} ({:.2e} of the size); faces identical {}/{}",
        level.level,
        level_index + 1,
        m.levels.len(),
        mesh.positions.len(),
        mesh.faces.len(),
        worst,
        relative,
        identical,
        mesh.faces.len()
    );
    totals.parts += 1;
    totals.vertices += mesh.positions.len() as u64;
    totals.faces += mesh.faces.len() as u64;
    totals.faces_identical += identical as u64;
    totals.worst_error = totals.worst_error.max(worst);
    totals.worst_relative = totals.worst_relative.max(relative);
    *totals.by_level.entry(level.level).or_default() += 1;
    let mut parts = vec![line];
    extras(m, level, obj, base, totals, &mut parts);
    parts.join("; ")
}

/// Groups, UVs and mask and color lines, when both sides have them.
fn extras(
    m: &gs_io::ztl::MeshData,
    level: &gs_io::ztl::Level,
    obj: &Obj,
    base: (usize, usize),
    totals: &mut Totals,
    parts: &mut Vec<String>,
) {
    let mesh = &level.mesh;
    if let Some(AttrValues::U32(groups)) = mesh.attributes.get("group").map(|a| &a.values) {
        let theirs = &obj.groups[base.1..base.1 + mesh.faces.len()];
        let same = same_partition(groups, theirs);
        totals.groups_checked += 1;
        totals.groups_matching += u64::from(same);
        parts.push(format!("groups match: {same}"));
    }
    if let (Some(uvs), false, 0) = (&m.uvs, obj.uvs.is_empty(), level.level) {
        let (mut all, mut ok) = (0u64, 0u64);
        for (f, uv) in uvs.iter().enumerate() {
            for (k, c) in obj.faces[base.1 + f].iter().enumerate() {
                let Some(t) = obj.uvs.get(c.1 as usize) else {
                    continue;
                };
                all += 1;
                let close = (f64::from(uv[k][0]) - t[0]).abs() < 1e-4
                    && (f64::from(uv[k][1]) - t[1]).abs() < 1e-4;
                ok += u64::from(close);
            }
        }
        totals.uv_corners += all;
        totals.uv_corners_matching += ok;
        parts.push(format!("uv corners {ok}/{all}"));
    }
    if obj.mrgb.len() == obj.positions.len() {
        let (mut mask_ok, mut color_ok) = (0u64, 0u64);
        for (i, d) in level.vertex_data.iter().enumerate() {
            let t = obj.mrgb[base.0 + i];
            mask_ok += u64::from(d[2] == t[0] && d[3] == t[0]);
            color_ok += u64::from([d[6], d[5], d[4]] == [t[1], t[2], t[3]]);
        }
        let n = level.vertex_data.len() as u64;
        totals.mrgb_vertices += n;
        totals.mrgb_mask_matching += mask_ok;
        totals.mrgb_color_matching += color_ok;
        parts.push(format!("MRGB mask {mask_ok}/{n}, color {color_ok}/{n}"));
    }
}

/// True if two face labellings group the faces the same way.
fn same_partition(a: &[u32], b: &[u32]) -> bool {
    let mut ab: BTreeMap<u32, u32> = BTreeMap::new();
    let mut ba: BTreeMap<u32, u32> = BTreeMap::new();
    a.iter()
        .zip(b)
        .all(|(&x, &y)| *ab.entry(x).or_insert(y) == y && *ba.entry(y).or_insert(x) == x)
}

fn check_pair(ztl: &str, obj_path: &str, totals: &mut Totals) -> Result<Vec<String>, String> {
    let obj = parse_obj(obj_path).map_err(|e| e.to_string())?;
    let source = File::open(ztl).map_err(|e| e.to_string())?;
    let mut file = open(BufReader::new(source)).map_err(|e| e.to_string())?;
    let mut cands = Vec::new();
    let mut slots = Vec::new();
    for t in 0..file.tools.len() {
        for s in 0..file.tools[t].parts.len() {
            let g = file.geometry(t, s).map_err(|e| e.to_string())?;
            let levels = g.mesh.as_ref().map_or(&[][..], |m| &m.levels[..]);
            cands.push(
                levels
                    .iter()
                    .enumerate()
                    .map(|(i, l)| Candidate {
                        tool: t,
                        part: s,
                        level_index: i,
                        vertices: l.mesh.positions.len(),
                        faces: l.mesh.faces.len(),
                        first: l.mesh.faces.iter().take(3).copied().collect(),
                    })
                    .collect::<Vec<_>>(),
            );
            slots.push((t, s));
        }
    }
    let Some(picked) = pick(&cands, obj.positions.len(), obj.faces.len()) else {
        return Err(format!(
            "no parts add up to {} vertices and {} faces",
            obj.positions.len(),
            obj.faces.len()
        ));
    };
    totals.matched += 1;
    let mut out = Vec::new();
    let mut base = (0, 0);
    for c in order(&picked, &obj) {
        let g = file.geometry(c.tool, c.part).map_err(|e| e.to_string())?;
        let line = compare(&g, c.level_index, &obj, base, totals);
        out.push(format!("    tool {} part {}: {line}", c.tool, c.part));
        base = (base.0 + c.vertices, base.1 + c.faces);
    }
    Ok(out)
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(pairs) = args.first() else {
        return Err(std::io::Error::other(
            "usage: ztl_objcheck <pairs file> [report file]",
        ));
    };
    let mut totals = Totals::default();
    let mut out = vec![String::from("# OBJ check\n")];
    for line in BufReader::new(File::open(pairs)?).lines() {
        let line = line?;
        let Some((ztl, obj)) = line.split_once('\t') else {
            continue;
        };
        totals.pairs += 1;
        let result = check_pair(ztl, obj, &mut totals);
        out.push(obj.to_string());
        match result {
            Ok(lines) => out.extend(lines),
            Err(e) => {
                out.push(format!("    not matched: {e}"));
            }
        }
    }
    let t = &totals;
    out.push(format!(
        "\n## totals\n\npairs {}, matched {}\npart levels compared {} ({} vertices, {} faces), {} without a stored export scale\nby level: {:?}\nfaces identical {} of {}\nworst position error {:.3e}, worst relative to model size {:.3e}\ngroup partitions matching {} of {}\nUV corners matching {} of {}\nMRGB vertices {}: mask matching {}, color matching {}",
        t.pairs,
        t.matched,
        t.parts,
        t.vertices,
        t.faces,
        t.no_export,
        t.by_level,
        t.faces_identical,
        t.faces,
        t.worst_error,
        t.worst_relative,
        t.groups_matching,
        t.groups_checked,
        t.uv_corners_matching,
        t.uv_corners,
        t.mrgb_vertices,
        t.mrgb_mask_matching,
        t.mrgb_color_matching
    ));
    let out = out.join("\n") + "\n";
    match args.get(1) {
        Some(file) => std::fs::write(file, out),
        None => std::io::stdout().lock().write_all(out.as_bytes()),
    }
}
