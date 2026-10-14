//! Tool and project file survey (not a gate): opens every .ZTL and .ZPR file under a folder with
//! `gs_io::ztl::open`, decodes every part one at a time and reports totals: versions, part
//! counts, list entries, levels, faces, UVs, groups, masks, colors, slots not decoded, setting
//! names, and each failure with its reason.
//!
//! ```text
//! cargo run --release -p gs-io --example ztl_survey -- <folder> [report file]
//! ```
//!
//! Without a report file the report goes to standard output. The report names failing files
//! (it stays outside the repository); `docs/decisions/0005-ztl-format.md` copies totals only.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufReader, Write as _};
use std::path::{Path, PathBuf};

use gs_io::ztl::{FileKind, Geometry, Level, MeshData, PartInfo, Tool, ZtlFile, open};
use gs_protocol::{AttrValues, NO_VERTEX};

/// Counts of anything printable, in sorted order.
type Tally = BTreeMap<String, u64>;

#[derive(Default)]
struct Survey {
    files: u64,
    opened: u64,
    bytes: u64,
    parts: u64,
    decoded: u64,
    failures: Vec<(PathBuf, String)>,
    tallies: BTreeMap<&'static str, Tally>,
    sums: BTreeMap<&'static str, u64>,
}

impl Survey {
    fn count<K: ToString + ?Sized>(&mut self, table: &'static str, key: &K) {
        self.add(table, key, 1);
    }

    fn add<K: ToString + ?Sized>(&mut self, table: &'static str, key: &K, n: u64) {
        *self
            .tallies
            .entry(table)
            .or_default()
            .entry(key.to_string())
            .or_default() += n;
    }

    fn sum(&mut self, name: &'static str, n: u64) {
        *self.sums.entry(name).or_default() += n;
    }

    fn file<R: std::io::Read + std::io::Seek>(&mut self, path: &Path, file: &mut ZtlFile<R>) {
        self.opened += 1;
        let h = &file.header;
        let kind = match h.kind {
            FileKind::Tool => "tool",
            FileKind::Project => "project",
        };
        self.count("file kind", kind);
        self.count("preamble format", &h.preamble.format);
        self.count("writer version text", &format!("{:?}", h.preamble.writer));
        let icon = &h.icon.image;
        let key = format!("{}x{} x{}", icon.width, icon.height, icon.bytes_per_pixel);
        self.count("icon", &key);
        if let Some(p) = &h.project {
            self.count("project object version", &p.version);
            let tags: Vec<String> = p.sections.iter().map(|(t, _)| format!("{t:#x}")).collect();
            self.count("project section tags", &tags.join(" "));
        }
        self.count("tools per file", &file.tools.len());
        let tools = file.tools.clone();
        for (t, tool) in tools.iter().enumerate() {
            self.tool(tool);
            for s in 0..tool.parts.len() {
                self.parts += 1;
                match file.geometry(t, s) {
                    Ok(g) => {
                        self.decoded += 1;
                        self.geometry(&g);
                        self.named_settings(&tool.parts[s], &g);
                    }
                    Err(e) => {
                        self.count("geometry failures", &e.to_string());
                        let what = format!("part {s} of tool {t}: {e}");
                        self.failures.push((path.to_path_buf(), what));
                    }
                }
            }
        }
    }

    fn tool(&mut self, tool: &Tool) {
        self.count("parts per tool", &tool.parts.len());
        self.count("tool head length", &tool.head.len());
        self.count("project tool flag", &format!("{:?}", tool.project_flag));
        self.count("selected part is first in list", &(tool.selected == 0));
        for s in &tool.parts {
            self.count("record version", &s.record_version);
            self.count("record head length", &s.record_head.len());
            self.count("lead byte / type", &format!("{} / {}", s.lead, s.kind));
            self.count("entry flags", &format!("{:#x}", s.entry.flags));
            self.count("entry color", &format!("{:02x?}", s.entry.color));
            self.count(
                "entry rest length / nonzero bytes",
                &format!(
                    "{} / {}",
                    s.entry.rest.len(),
                    s.entry.rest.iter().filter(|&&b| b != 0).count()
                ),
            );
            self.count("name object equals entry name", &(s.name == s.entry.name));
            self.count("settings head length", &s.settings.head.len());
            self.count("settings records", &s.settings.records.len());
            for r in &s.settings.records {
                self.count("setting kinds", &format!("{:?}", r.kind));
            }
        }
    }

    /// How a few named settings compare with the geometry: leads for what they mean.
    fn named_settings(&mut self, info: &PartInfo, g: &Geometry) {
        let find = |name: &str| info.settings.records.iter().find(|r| r.name == name);
        let Some(m) = &g.mesh else {
            return;
        };
        if let Some(sdiv) = find("SDiv") {
            let key = format!(
                "SDiv value / current level / level count: {} / {} / {}",
                sdiv.value, m.current_level, m.level_count
            );
            let matches = (sdiv.value - (m.current_level + 1) as f32).abs() < 0.5;
            self.count("SDiv equals current level + 1", &matches);
            if !matches {
                self.count("SDiv mismatches", &key);
            }
        }
        if let Some(colorize) = find("Colorize") {
            let painted = m.levels.iter().any(|l| {
                l.vertex_data.iter().any(|d| {
                    d[4..7] != [0xFF, 0xFF, 0xFF]
                        && d[4..7] != [0xFE, 0xFE, 0xFE]
                        && d[4..7] != [0, 0, 0]
                })
            });
            let key = format!(
                "Colorize on {} / colors other than white, grey 254 or black {}",
                colorize.on, painted
            );
            self.count("Colorize switch against stored colors", &key);
        }
    }

    fn geometry(&mut self, g: &Geometry) {
        let versions = format!("{} + {:?}", g.mesh_version, g.companion_version);
        self.count("mesh / companion version", &versions);
        self.count(
            "mesh head / companion head lengths",
            &format!("{} / {}", g.mesh_head.len(), g.companion_head.len()),
        );
        for slot in &g.other {
            let objects: Vec<String> = slot
                .sections
                .iter()
                .map(|s| s.version.to_string())
                .collect();
            let key = format!(
                "{} v{} slot {}: objects [{}] {}",
                if slot.companion { "companion" } else { "mesh" },
                if slot.companion {
                    g.companion_version.unwrap_or(0)
                } else {
                    g.mesh_version
                },
                slot.index,
                objects.join(","),
                slot.array.map_or(String::new(), |a| format!(
                    "array of {}-byte elements",
                    a.element
                )),
            );
            self.count("slots not decoded", &key);
        }
        match &g.mesh {
            Some(m) => self.mesh(m),
            None => self.count("polygon mesh", "no"),
        }
    }

    fn mesh(&mut self, m: &MeshData) {
        self.count("polygon mesh", "yes");
        let tris = m.base_faces().iter().filter(|f| f[3] == NO_VERTEX).count() as u64;
        self.sum("base faces", m.base_faces().len() as u64);
        self.sum("base triangles", tris);
        self.count(
            "base faces kinds",
            match (tris, m.base_faces().len() as u64 - tris) {
                (0, _) => "quads only",
                (_, 0) => "triangles only",
                _ => "quads and triangles",
            },
        );
        self.count("level count", &m.level_count);
        self.count(
            "current level / level count",
            &format!("{} / {}", m.current_level, m.level_count),
        );
        if tris > 0 && m.level_count > 1 {
            self.count("triangles in a base with levels", &m.level_count);
        }
        self.count("stored levels", &m.levels.len());
        self.count("uvs", &m.uvs.is_some());
        let s = m.export.map_or([0.0; 3], |e| e.scale);
        self.count(
            "export scale is 1",
            &s.iter().all(|v| v.to_bits() == 1f32.to_bits()),
        );
        if let Some(data) = &m.face_data {
            let mut groups: Vec<u32> = data.iter().map(|v| v & 0xFFFF).collect();
            groups.sort_unstable();
            groups.dedup();
            self.count("groups per mesh", &bucket(groups.len() as u64));
            let mut flags: BTreeMap<u32, u64> = BTreeMap::new();
            for v in data {
                *flags.entry(v >> 16).or_default() += 1;
            }
            for (bits, n) in flags {
                self.add("face flag bits (above the group)", &format!("{bits:#x}"), n);
            }
        }
        for level in &m.levels {
            self.level(level);
        }
    }

    fn level(&mut self, level: &Level) {
        {
            self.sum("stored level vertices", level.mesh.positions.len() as u64);
            self.sum("stored level faces", level.mesh.faces.len() as u64);
            let mut hist = [[0u64; 256]; 3];
            for d in &level.vertex_data {
                hist[0][usize::from(d[0])] += 1;
                hist[1][usize::from(d[1])] += 1;
                hist[2][usize::from(d[7])] += 1;
            }
            for (table, h) in [
                "vertex data byte 0",
                "vertex data byte 1",
                "vertex data byte 7",
            ]
            .into_iter()
            .zip(hist)
            {
                for (value, n) in h.iter().enumerate().filter(|(_, n)| **n > 0) {
                    self.add(table, &value, *n);
                }
            }
            if let Some(AttrValues::F32(mask)) =
                level.mesh.attributes.get("mask").map(|a| &a.values)
            {
                self.count("meshes with a mask", "yes");
                let masked = mask.iter().filter(|&&v| v > 0.0).count() as u64;
                let partial = mask.iter().filter(|&&v| v > 0.0 && v < 1.0).count() as u64;
                self.sum("masked vertices", masked);
                self.sum("partly masked vertices", partial);
            }
            if level.mesh.attributes.contains_key("color") {
                self.count("meshes with color", "yes");
                self.color_order(&level.vertex_data);
            }
        }
    }

    /// Which stored color byte is largest on painted vertices that are not grey: the evidence
    /// for the channel order.
    fn color_order(&mut self, data: &[[u8; 8]]) {
        let mut seen = [[0u64; 3]; 3];
        for d in data {
            let c = [d[4], d[5], d[6]];
            if c[0] == c[1] && c[1] == c[2] {
                continue;
            }
            let max = (0..3).max_by_key(|&i| (c[i], i)).unwrap_or(0);
            let min = (0..3).min_by_key(|&i| (c[i], 2 - i)).unwrap_or(0);
            seen[max][min] += 1;
        }
        for (max, row) in seen.iter().enumerate() {
            for (min, &n) in row.iter().enumerate().filter(|(_, n)| **n > 0) {
                let key = format!("byte {} / byte {}", 4 + max, 4 + min);
                self.add("colored vertex: largest / smallest stored byte", &key, n);
            }
        }
    }

    fn report(&self) -> Result<String, std::fmt::Error> {
        let mut out = String::new();
        writeln!(
            out,
            "# Tool and project file survey\n\nfiles {}, {} bytes, opened {}, failed {}\nparts {}, decoded {}\n",
            self.files,
            self.bytes,
            self.opened,
            self.failures.len(),
            self.parts,
            self.decoded
        )?;
        writeln!(out, "## sums")?;
        for (name, n) in &self.sums {
            writeln!(out, "  {n:>14}  {name}")?;
        }
        writeln!(out)?;
        for (table, tally) in &self.tallies {
            writeln!(out, "## {table}")?;
            for (key, n) in tally {
                writeln!(out, "  {n:>10}  {key}")?;
            }
            writeln!(out)?;
        }
        writeln!(out, "## failures\n")?;
        for (path, reason) in &self.failures {
            writeln!(out, "  {}: {reason}", path.display())?;
        }
        Ok(out)
    }
}

/// A rough size class for counts.
fn bucket(n: u64) -> String {
    match n {
        0..=9 => n.to_string(),
        10..=99 => "10 to 99".into(),
        100..=999 => "100 to 999".into(),
        _ => "1000 or more".into(),
    }
}

/// Every file under `dir` whose extension is .ztl or .zpr in any case, folders depth first,
/// in sorted order.
fn scene_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let wanted = |p: &Path| {
        p.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("ztl") || e.eq_ignore_ascii_case("zpr"))
    };
    let mut out = Vec::new();
    let mut folders = vec![dir.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&folder)?
            .map(|e| e.map(|e| e.path()))
            .collect::<Result<_, _>>()?;
        entries.sort();
        let (subfolders, files): (Vec<PathBuf>, Vec<PathBuf>) =
            entries.into_iter().partition(|p| p.is_dir());
        out.extend(files.into_iter().filter(|p| wanted(p)));
        folders.extend(subfolders.into_iter().rev());
    }
    Ok(out)
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(dir) = args.first() else {
        return Err(std::io::Error::other(
            "usage: ztl_survey <folder> [report file]",
        ));
    };
    let files = scene_files(Path::new(dir))?;
    let mut survey = Survey::default();
    for path in files {
        survey.files += 1;
        let source = File::open(&path)?;
        survey.bytes += source.metadata()?.len();
        match open(BufReader::new(source)) {
            Ok(mut file) => survey.file(&path, &mut file),
            Err(err) => survey.failures.push((path, err.to_string())),
        }
    }
    let report = survey.report().map_err(std::io::Error::other)?;
    match args.get(1) {
        Some(file) => std::fs::write(file, report),
        None => std::io::stdout().lock().write_all(report.as_bytes()),
    }
}
