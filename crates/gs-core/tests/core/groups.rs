//! Face groups (decision 0026): the largest-id summary against a brute-force scan and the group
//! column against a recorded model after every edit, undo, redo and edge split (differential and
//! model), the same edits giving the same ids on 1, 8 and 24 threads (determinism), a fresh id
//! held to one summary read per node (work counter), and the colors pinned to their bytes.

use gs_core::groups::{GroupMaker, MAX_GROUP, NoFreshId, fresh_after, group_color};
use gs_core::mask::Scope;
use gs_core::select::{FaceRule, Selector};
use gs_core::{MeshStore, NO, Vec3, View, VisibilityEdit};
use gs_protocol::{AttrValues, PlainMesh};

use crate::brush_kit::floats;
use crate::common::{mix, refine, small_nodes, sphere, store, surface_point, unit};

/// The exported group column (ids are small, so exact as floats).
fn groups(m: &PlainMesh) -> Vec<f32> {
    floats(m, "group")
}

/// The largest id a live face holds, read face by face.
fn brute_largest(m: &MeshStore) -> u32 {
    let Some(g) = m.attr_id("group") else {
        return 0;
    };
    let r = m.attr_reader(g);
    let mut largest = 0;
    for n in 0..m.node_count() as u32 {
        let node = m.node(n);
        for (j, f) in node.faces.iter().enumerate() {
            if f[0] != NO {
                largest = largest.max(r.label(node.first_face + j as u32));
            }
        }
    }
    largest
}

/// One random group edit, as one history step, picked by `h`. Returns whether the step was kept
/// (a step that changed nothing is dropped).
fn edit(m: &mut MeshStore, h: u64) -> bool {
    let masked = |h: u64| Selector::Masked {
        threshold: 0.2 + unit(h).abs() * 0.6,
        rule: [FaceRule::Mean, FaceRule::Any, FaceRule::All][(h >> 8) as usize % 3],
    };
    m.begin_step();
    match h % 8 {
        0 => {
            let id = [0, 3, 40, m.largest_group()][(h >> 16) as usize % 4];
            let set = m.select(&masked(h));
            m.assign_group(&set, id);
        }
        1 => {
            let sources = [(h >> 4) as u32 % 6, (h >> 12) as u32 % 6];
            m.merge_groups(&sources, (h >> 20) as u32 % 4);
        }
        2 => drop(m.make_groups(GroupMaker::Shells)),
        3 => drop(m.make_groups(GroupMaker::Normals { max_angle: 20.0 })),
        4 => drop(m.make_groups(GroupMaker::Visible {
            coverage: 0.4,
            cluster: unit(h).abs(),
            seed: h >> 32,
        })),
        5 => drop(m.make_groups(GroupMaker::Masked {
            clear_mask: h & 32 != 0,
        })),
        6 => {
            let view = View {
                dir: Vec3::new(unit(h), unit(mix(h)), -1.0).normalized(),
                eye: None,
            };
            drop(m.make_groups(GroupMaker::Front {
                view,
                angle: 60.0,
                additive: h & 64 != 0,
            }));
        }
        _ => {
            let set = m.select(&masked(h));
            m.edit_visibility(VisibilityEdit::Hide(&set));
        }
    }
    m.end_step().nodes > 0
}

#[test]
fn the_largest_id_and_the_group_column_hold_through_every_edit_undo_redo_and_split() {
    let mut m = store(&sphere(14, 1.0, 3, true), small_nodes());
    // The group column the store showed at each undo depth, the model undo and redo must match.
    let mut model = vec![groups(&m.to_plain())];
    for step in 0..80u64 {
        let h = mix(step * 7919 + 5);
        let what = format!("step {step} (op {})", h % 11);
        let kept = match h % 11 {
            // Undo and redo move along the model; they never add to it.
            8 => {
                drop(m.undo());
                false
            }
            9 => {
                drop(m.redo());
                false
            }
            10 => {
                let c = surface_point(&m, Vec3::new(unit(h), unit(mix(h)), 0.6));
                m.begin_step();
                refine(&mut m, c, 0.35, 0.04, 2);
                if h & 128 != 0 {
                    m.mask_set(unit(mix(h ^ 9)).abs(), Scope::All);
                }
                m.end_step().nodes > 0
            }
            _ => edit(&mut m, h >> 4),
        };
        let (depth, now) = (m.undo_len(), groups(&m.to_plain()));
        if kept {
            model.truncate(depth);
            model.push(now.clone());
        }
        assert!(
            depth < model.len(),
            "{what}: undo depth {depth} past the model"
        );
        assert_eq!(now, model[depth], "{what}: group column against the model");
        assert_eq!(m.largest_group(), brute_largest(&m), "{what}: largest id");
    }
}

#[test]
fn the_same_edits_give_the_same_ids_on_1_8_and_24_threads() {
    let script = || {
        let mut m = store(&sphere(14, 0.5, 9, true), small_nodes());
        (0..24u64)
            .map(|step| {
                edit(&mut m, mix(step * 31 + 2));
                groups(&m.to_plain())
            })
            .collect::<Vec<_>>()
    };
    let runs = [1, 8, 24].map(|t| gs_par::with_threads(t, script));
    assert_eq!(runs[0], runs[1], "1 and 8 threads");
    assert_eq!(runs[0], runs[2], "1 and 24 threads");
    assert!(
        runs[0].last().is_some_and(|g| g.iter().any(|&id| id > 5.0)),
        "the script made groups"
    );
}

#[test]
fn a_fresh_id_reads_each_node_summary_once() {
    let mut m = store(&sphere(16, 1.0, 4, true), small_nodes());
    let nodes = m.node_count() as u64;
    assert!(nodes > 20, "a part of many nodes");
    let reads = |m: &MeshStore| m.work().group_reads;
    let before = reads(&m);
    let fresh = m.fresh_group();
    assert_eq!(reads(&m) - before, nodes, "one read per node summary");
    assert_eq!(fresh, Ok(brute_largest(&m) + 1), "one above the largest id");
    // A maker takes one fresh id however many groups it makes; a given id reads nothing.
    let before = reads(&m);
    let made = m.make_groups(GroupMaker::Normals { max_angle: 5.0 });
    let [first, last] = made.ok().and_then(|m| m.ids).unwrap_or_default();
    assert!(last > first, "the maker made several groups");
    assert_eq!(reads(&m) - before, nodes, "and read each summary once");
    let before = reads(&m);
    let set = m.select(&Selector::All);
    m.assign_group(&set, 2);
    assert_eq!(reads(&m), before, "assigning a given id reads no summary");
}

/// Fresh ids stop at [`MAX_GROUP`] (review 0007, m1: the id overflowed past `u32::MAX`, or
/// wrapped to 0, no group): `fresh_after` answers `None` rather than pass it, a maker that would
/// pass it writes nothing, and a mesh holding a larger id is refused as it comes in.
#[test]
fn fresh_ids_stop_at_the_ceiling_and_a_maker_past_it_writes_nothing() {
    assert_eq!(fresh_after(0, 1), Some(1));
    assert_eq!(fresh_after(MAX_GROUP - 2, 2), Some(MAX_GROUP - 1));
    assert_eq!(fresh_after(MAX_GROUP - 1, 2), None, "two ids, one left");
    for largest in [MAX_GROUP, u32::MAX] {
        assert_eq!(fresh_after(largest, 1), None, "{largest}");
    }
    let mut m = store(&sphere(16, 1.0, 4, true), small_nodes());
    let all = m.select(&Selector::All);
    m.assign_group(&all, MAX_GROUP);
    let column = |m: &MeshStore| match &m.to_plain().attributes["group"].values {
        AttrValues::U32(v) => v.clone(),
        AttrValues::F32(_) => Vec::new(),
    };
    let before = column(&m);
    let full = NoFreshId {
        largest: MAX_GROUP,
        wanted: 1,
    };
    assert_eq!(m.fresh_group(), Err(full));
    assert_eq!(m.make_groups(GroupMaker::Shells), Err(full));
    assert_eq!(column(&m), before, "nothing written");
    let mut held = m.to_plain();
    if let Some(a) = held.attributes.get_mut("group") {
        a.values = AttrValues::U32(vec![MAX_GROUP + 1; held.faces.len()]);
    }
    assert!(MeshStore::from_plain(&held, small_nodes()).is_err());
}

/// A color's hue in degrees.
fn hue([r, g, b, _]: [u8; 4]) -> f64 {
    let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
    let d = f64::from(hi - lo);
    let [x, y, z] = [r, g, b].map(f64::from);
    let sector = if hi == r {
        (y - z) / d
    } else if hi == g {
        (z - x) / d + 2.0
    } else {
        (x - y) / d + 4.0
    };
    (sector * 60.0).rem_euclid(360.0)
}

#[test]
fn colors_are_pinned_bytes_and_consecutive_ids_turn_at_least_30_degrees() {
    // The bytes every machine must give: Blender's face set colors for the same ids and seeds
    // (decision 0039, 9), worked out apart from the code from Blender's source with numpy's
    // float32 arithmetic.
    let pinned = [
        ((0, 0), [255, 255, 255, 255]),
        ((1, 0), [60, 93, 175, 255]),
        ((2, 0), [126, 192, 34, 255]),
        ((3, 0), [209, 46, 189, 255]),
        ((1, 7), [203, 55, 104, 255]),
        ((1000, 0), [228, 97, 63, 255]),
        ((5, u32::MAX), [184, 196, 53, 255]),
    ];
    for ((id, seed), want) in pinned {
        assert_eq!(group_color(id, seed), want, "group {id}, seed {seed}");
    }
    for seed in [0, 7, u32::MAX] {
        for id in 1..2000 {
            let turn = (hue(group_color(id + 1, seed)) - hue(group_color(id, seed))).abs();
            let apart = turn.min(360.0 - turn);
            assert!(
                apart >= 30.0,
                "ids {id} and {} seed {seed}: {apart}",
                id + 1
            );
        }
    }
}
