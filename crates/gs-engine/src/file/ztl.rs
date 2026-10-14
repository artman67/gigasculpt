//! ZTL tools (.ztl) and ZPR projects (.zpr) as parts (decision 0005): one part at a time, so
//! only the part being read sits in memory beside the scene.
//!
//! Each part in the file becomes a part of the scene with its detail levels (decision 0014): the
//! top stored level is the top, the level it was saved at is active, with its stored points as
//! edits not carried up yet when it is below the top. Face groups, mask, colors and UVs come
//! along (UVs carried from the base faces up the levels with the numbering of decision 0006).

use std::fs::File;
use std::io::BufReader;

use gs_io::ReadError;
use gs_io::mesh::UV_COLUMNS;
use gs_io::ztl::{FileKind, MeshData};
use gs_protocol::{AttrDomain, AttrValues, Attribute, CommandError, ErrorCode};

use super::import::{Levels, Object, Run, Stack};
use crate::document::DocTxn;
use crate::errors::{self, error};
use crate::job::JobCell;

/// Imports every part of the tools `run.args.tool` picks into `doc`, reporting to `job`.
pub(super) fn import(
    file: File,
    run: &mut Run,
    (doc, job): (&mut DocTxn, Option<&JobCell>),
    unreadable: &dyn Fn(ReadError) -> CommandError,
) -> Result<(), CommandError> {
    let mut z = gs_io::ztl::open(BufReader::new(file)).map_err(unreadable)?;
    let project = z.header.kind == FileKind::Project;
    let tools: Vec<usize> = match run.args.tool {
        Some(t) if (t as usize) < z.tools.len() => vec![t as usize],
        Some(_) => {
            let n = z.tools.len();
            return Err(errors::args_invalid(
                "/tool",
                format!("the file holds {n} tools, numbered 0 to {}", n - 1),
            ));
        }
        None => (0..z.tools.len()).collect(),
    };
    let jobs: Vec<(usize, usize)> = (tools.iter())
        .flat_map(|&t| (0..z.tools[t].parts.len()).map(move |s| (t, s)))
        .collect();
    run.objects = jobs.len();
    for (i, &(t, s)) in jobs.iter().enumerate() {
        if job.is_some_and(JobCell::cancelled) {
            return Err(error(ErrorCode::Cancelled, "the import was cancelled"));
        }
        let info = &z.tools[t].parts[s];
        let name = if info.entry.name.is_empty() {
            info.name.clone()
        } else {
            info.entry.name.clone()
        };
        let doing = format!("reading part {name} ({i} of {} done)", jobs.len());
        if let Some(job) = job {
            job.progress(None, doing);
        }
        let geometry = z.geometry(t, s).map_err(unreadable)?;
        if !geometry.other.is_empty() {
            run.notes.push(format!(
                "{name}: {} stored parts not read (3D layers, morph targets or data not decoded \
                 yet)",
                geometry.other.len()
            ));
        }
        let Some(mesh) = geometry.mesh else {
            run.notes.push(format!(
                "{name}: no polygon mesh (a sphere armature or a parametric primitive), not \
                 imported"
            ));
            continue;
        };
        let object = as_object(mesh, name, project.then_some(t as u32), run);
        run.add(doc, object)?;
    }
    Ok(())
}

/// One part as an object: its top stored level with its UVs and export transform, and when it
/// has detail levels, what keeps them (decision 0014): the base faces, and the level it was
/// saved at when that is below the top, as stored.
fn as_object(mut m: MeshData, source: String, tool: Option<u32>, run: &mut Run) -> Object {
    let top = m.levels.last().map_or(0, |l| l.level);
    let uvs =
        (m.uvs.take()).map(|uvs| gs_core::Levels::linear_uvs(m.base_faces(), uvs, top as usize));
    let stored: Vec<u32> = m.levels.iter().map(|l| l.level).collect();
    let base = m.base_faces().to_vec();
    let mut mesh = m.levels.pop().map(|l| l.mesh).unwrap_or_default();
    let saved = (m.levels.pop())
        .filter(|l| l.level < top)
        .map(|l| (l.level as usize, l.mesh));
    if m.level_count > top + 1 {
        run.notes.push(format!(
            "{source}: levels above {top} are not stored in the file, so not kept"
        ));
    }
    if top > 0 {
        let shown = saved.as_ref().map_or(top as usize, |s| s.0);
        run.notes.push(format!(
            "{source}: kept its {} detail levels, level {shown} active{}; levels the file does \
             not store are the top level's first points (how the saving app rebuilds them is \
             not confirmed yet)",
            top + 1,
            if saved.is_some() {
                " with its stored points as edits not carried up yet"
            } else {
                ""
            },
        ));
    }
    if let Some(uvs) = uvs.filter(|c| c.len() == mesh.faces.len()) {
        if top > 0 {
            run.notes.push(format!(
                "{source}: UVs above the base level are carried up linearly (how the saving \
                 app smooths them is not known yet)"
            ));
        }
        for (k, name) in UV_COLUMNS.iter().enumerate() {
            let domain = AttrDomain::Face;
            let values = AttrValues::F32(uvs.iter().map(|c| c[k / 2][k % 2]).collect());
            mesh.attributes
                .insert((*name).to_string(), Attribute { domain, values });
        }
    }
    let export = m.export.map(|e| {
        if e.scale.iter().any(|&s| s.to_bits() != e.scale[0].to_bits()) {
            run.notes.push(format!(
                "{source}: the export scale differs per axis {:?}; the first is used",
                e.scale
            ));
        }
        (e.scale[0], e.offset)
    });
    let imported = saved.as_ref().map_or(top, |s| s.0 as u32);
    let stack = (top > 0).then(|| Stack {
        base,
        base_count: m.base_vertex_count,
        count: top as usize + 1,
        saved,
    });
    Object {
        source,
        tool,
        mesh,
        export,
        levels: Some(Levels {
            count: m.level_count,
            saved_at: m.current_level,
            stored,
            imported,
            not_kept: (top + 1..m.level_count).collect(),
        }),
        stack,
    }
}
