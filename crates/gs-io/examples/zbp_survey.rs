//! Brush file survey (not a gate): reads every .ZBP file under a folder with `gs_io::zbp::read`
//! and reports how many parse, the format and object versions, base brush types, payload kinds,
//! every setting name with counts and value ranges, and each failure with its reason.
//!
//! ```text
//! cargo run --release -p gs-io --example zbp_survey -- <folder> [report file]
//! ```
//!
//! Without a report file the report goes to standard output. The report names failing files;
//! the setting names and counts are what `docs/decisions/0004-zbp-format.md` copies.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use gs_io::zbp::{SettingKind, ZbpBrush, read};

/// Counts of anything printable, in sorted order.
type Tally = BTreeMap<String, usize>;

/// Value range and use of one setting (group, kind, name).
#[derive(Default)]
struct SettingStats {
    files: usize,
    on: usize,
    min: f32,
    max: f32,
    points: BTreeMap<usize, usize>,
}

#[derive(Default)]
struct Survey {
    files: usize,
    parsed: usize,
    failures: Vec<(PathBuf, String)>,
    tallies: BTreeMap<&'static str, Tally>,
    settings: BTreeMap<(u8, String, String), SettingStats>,
}

impl Survey {
    fn count<K: ToString + ?Sized>(&mut self, table: &'static str, key: &K) {
        *self
            .tallies
            .entry(table)
            .or_default()
            .entry(key.to_string())
            .or_default() += 1;
    }

    fn add(&mut self, brush: &ZbpBrush) {
        self.parsed += 1;
        self.count("preamble format", &brush.preamble.format);
        self.count(
            "writer version text",
            &format!("{:?}", brush.preamble.writer),
        );
        self.count("header version", &brush.header.version);
        self.count(
            "header length by version",
            &format!("{} / {}", brush.header.version, brush.header.raw.len()),
        );
        self.count("credit lines", &brush.header.credits.len());
        self.count("base type", &brush.header.base_type);
        let shape = (brush.alpha.is_some(), !brush.meshes.is_empty());
        self.count(
            "base type / alpha / meshes",
            &format!("{} / {} / {}", brush.header.base_type, shape.0, shape.1),
        );
        self.count(
            "brush object version",
            &format!("{:#x}", brush.head.version),
        );
        let versions = format!(
            "{:?} / {} / {:#x}",
            brush.preamble.writer, brush.header.version, brush.head.version
        );
        self.count("writer / header version / brush object version", &versions);
        self.count("draw size stored", &brush.head.draw_size.is_some());
        self.count("alpha flags", &format!("{:#x}", brush.head.alpha_flags));
        self.count("brush head unknown float", &brush.head.unknown);
        if let Some(last) = brush.blocks.last() {
            let word = |at: usize| {
                last.bytes
                    .get(at..at + 4)
                    .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            };
            self.count(
                "last block words 5 and 6",
                &format!("{:?} {:?}", word(20), word(24)),
            );
        }
        let icon = &brush.icon.image;
        self.count(
            "icon",
            &format!(
                "{}x{} x{} {:?}",
                icon.width, icon.height, icon.bytes_per_pixel, brush.icon.name
            ),
        );
        self.add_payloads(brush);
        self.add_settings(brush);
    }

    fn add_payloads(&mut self, brush: &ZbpBrush) {
        match &brush.alpha {
            Some(alpha) => {
                let image = &alpha.image;
                self.count("alpha bytes per pixel", &image.bytes_per_pixel);
                self.count("alpha size", &format!("{}x{}", image.width, image.height));
                self.count("alpha name", &format!("{:?}", alpha.name));
            }
            None => self.count("alpha bytes per pixel", &"none"),
        }
        self.count("mesh entries per brush", &brush.meshes.len());
        for mesh in &brush.meshes {
            let versions = format!("{:#x} + {:#x}", mesh.mesh.version, mesh.companion.version);
            self.count("mesh / companion object versions", &versions);
            self.count("mesh entry tag", &mesh.tag);
        }
        self.count("plain-field blocks per brush", &brush.blocks.len());
        for block in &brush.blocks {
            self.count("plain-field block lengths", &block.bytes.len());
        }
        self.count("sections per brush", &brush.sections.len());
        for section in &brush.sections {
            self.count(
                "section object versions",
                &format!("{:#x}", section.object.version),
            );
            for image in &section.images {
                let key = format!(
                    "{}x{} x{}",
                    image.width, image.height, image.bytes_per_pixel
                );
                self.count("images inside sections", &key);
            }
        }
    }

    fn add_settings(&mut self, brush: &ZbpBrush) {
        self.count("settings per brush", &brush.settings.len());
        for s in &brush.settings {
            self.count("setting kinds", &format!("{:?}", s.kind));
            self.count("setting second value", &s.second);
            self.count("setting head bytes", &format!("{:02x?}", s.head));
            if let Some(curve) = &s.curve {
                self.count("curve versions", &curve.version);
                self.count(
                    "curve head lengths",
                    &format!("v{} / {}", curve.version, curve.head.len()),
                );
                for p in &curve.points {
                    self.count("curve point kinds", &p.kind);
                    self.count("curve point weights", &p.weight);
                }
            }
            let key = (s.group, kind_name(s.kind), s.name.clone());
            let stats = self.settings.entry(key).or_default();
            if stats.files == 0 {
                (stats.min, stats.max) = (s.value, s.value);
            }
            stats.files += 1;
            stats.on += usize::from(s.on);
            stats.min = stats.min.min(s.value);
            stats.max = stats.max.max(s.value);
            if let Some(curve) = &s.curve {
                *stats.points.entry(curve.points.len()).or_default() += 1;
            }
        }
    }

    fn report(&self) -> Result<String, std::fmt::Error> {
        let mut out = String::new();
        let failed = self.failures.len();
        writeln!(
            out,
            "# Brush file survey\n\nfiles {}, parsed {}, failed {failed}\n",
            self.files, self.parsed
        )?;
        for (table, tally) in &self.tallies {
            writeln!(out, "## {table}")?;
            for (key, n) in tally {
                writeln!(out, "  {n:>6}  {key}")?;
            }
            writeln!(out)?;
        }
        writeln!(
            out,
            "## settings: group, kind, name, files, on, min, max, curve points\n"
        )?;
        for ((group, kind, name), s) in &self.settings {
            let points: Vec<String> = s.points.iter().map(|(p, n)| format!("{p}:{n}")).collect();
            writeln!(
                out,
                "  {group}  {kind:<6}  {name:<26} {:>5} {:>5} {:>10} {:>10}  {}",
                s.files,
                s.on,
                s.min,
                s.max,
                points.join(" ")
            )?;
        }
        writeln!(out, "\n## failures\n")?;
        for (path, reason) in &self.failures {
            writeln!(out, "  {}: {reason}", path.display())?;
        }
        Ok(out)
    }
}

fn kind_name(kind: SettingKind) -> String {
    match kind {
        SettingKind::Slider => "slider".into(),
        SettingKind::Switch => "switch".into(),
        SettingKind::Curve => "curve".into(),
        SettingKind::Other(b) => format!("{b:#04x}"),
    }
}

/// Every file under `dir` whose extension is .zbp in any case, in sorted order.
fn brush_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<_, _>>()?;
    entries.sort();
    for path in entries {
        if path.is_dir() {
            brush_files(&path, out)?;
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("zbp"))
        {
            out.push(path);
        }
    }
    Ok(())
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(dir) = args.first() else {
        return Err(std::io::Error::other(
            "usage: zbp_survey <folder> [report file]",
        ));
    };
    let mut files = Vec::new();
    brush_files(Path::new(dir), &mut files)?;
    let mut survey = Survey::default();
    for path in files {
        survey.files += 1;
        match read(&std::fs::read(&path)?) {
            Ok(brush) => survey.add(&brush),
            Err(err) => survey.failures.push((path, err.to_string())),
        }
    }
    let report = survey.report().map_err(std::io::Error::other)?;
    match args.get(1) {
        Some(file) => std::fs::write(file, report),
        None => std::io::stdout().lock().write_all(report.as_bytes()),
    }
}
