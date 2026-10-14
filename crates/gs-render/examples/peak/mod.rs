//! The measuring tools' peak memory, shared with gs-engine's `examples/deform_preview.rs` and
//! `examples/import_survey.rs`.

/// The process's peak resident memory in KiB, as Linux reports it (`VmHWM`), or 0 unread.
pub fn peak_kib() -> f64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let kib = (status.lines())
        .find_map(|l| l.strip_prefix("VmHWM:"))
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<f64>().ok());
    kib.unwrap_or_default()
}
