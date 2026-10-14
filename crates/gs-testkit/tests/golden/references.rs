//! The repository's golden scenes against their approved references.

use crate::common::lavapipe;
use gs_testkit::golden::{Goldens, Status};

/// Every scene renders on lavapipe exactly as approved. A picture that does not pass is in
/// `target/golden/pending/` with its difference map in `target/golden/failures/<name>/`; see
/// `cargo run -p gs-testkit --bin golden -- review`.
#[test]
fn every_scene_matches_its_reference() {
    let g = Goldens::repo();
    let outcomes = g.check(lavapipe()).expect("every scene renders");
    let failed: Vec<String> = (outcomes.iter())
        .filter(|o| !matches!(o.status, Status::Matches(_)))
        .map(|o| format!("{}: {:?}", o.name, o.status))
        .collect();
    assert!(
        failed.is_empty(),
        "pictures that differ from their references (queued in {}):\n{}",
        g.pending().display(),
        failed.join("\n")
    );
    assert!(outcomes.len() >= 6, "at least three scenes in two looks");
}

/// Slow checks ARCHITECTURE 12.3 puts after tier 1: the gate's tier 1 skips every test in a
/// module named `tier2`, tier 2 runs them.
mod tier2 {
    use crate::common::hardware;
    use gs_testkit::golden::{GoldenScene, Goldens, compare, read, render};

    /// Each reference also passes, by the cross-renderer rule, when its scene is rendered on the
    /// real GPUs: their noise stays under the thresholds (ARCHITECTURE 10.3 asks this of every
    /// new scene). The worst numbers are printed for the record.
    #[test]
    fn references_hold_on_other_gpus() {
        let g = Goldens::repo();
        let mut failed = Vec::new();
        for scene in GoldenScene::load_all(&g.scenes()).expect("scenes load") {
            let parts = scene.run().expect("scene runs");
            for look in &scene.looks {
                let name = scene.picture(look);
                let reference = read(&g.reference(), &name)
                    .expect("reference reads")
                    .unwrap_or_else(|| panic!("{name} has a reference"));
                for (kind, gpu) in hardware() {
                    let image =
                        render(gpu, &parts, scene.how(), look, &g.matcaps).expect("renders");
                    let zebra = reference.manifest.is_zebra();
                    let verdict = compare(&reference.image, &image, false, zebra);
                    let line = format!("{name} on {kind:?}: {}", verdict.summary());
                    eprintln!("{line}");
                    if !verdict.pass {
                        failed.push(line);
                    }
                }
            }
        }
        assert!(
            failed.is_empty(),
            "scenes too noisy across GPUs:\n{}",
            failed.join("\n")
        );
    }
}
