//! Surface paint's color column in the store (decision 0039, section 2): a point nobody painted
//! reads white wherever a column is filled from nothing.

use gs_core::{AttrDef, AttrVec, ColumnSet};
use gs_protocol::AttrValues;

use crate::common::{ok, small_nodes, sphere, store};

const WHITE: u32 = u32::MAX;
const RED: u32 = 0xff00_00ff;

#[test]
fn a_part_painted_at_one_point_exports_white_everywhere_else() {
    let mut s = store(&sphere(16, 0.3, 7, false), small_nodes());
    assert!(s.node_count() > 4, "{} nodes", s.node_count());
    let col = ok(s.add_attr(AttrDef::color()));
    // As a stroke's first write to a node: the node's column is made, then one point painted.
    s.write(&[0], ColumnSet::attr(col)).each(|m| {
        if let AttrVec::U32(v) = m.attr(col) {
            v[0] = RED;
        }
    });
    let mesh = s.to_plain();
    let Some(AttrValues::U32(colors)) = mesh.attributes.get("color").map(|a| &a.values) else {
        panic!("the color column exports");
    };
    assert_eq!(colors.iter().filter(|&&c| c == RED).count(), 1, "one red");
    assert!(
        colors.iter().all(|&c| c == RED || c == WHITE),
        "every other point white, in the painted node and in the others"
    );
    // A node holding an all-white column holds the same content as one holding none.
    let mut whiter = s.share();
    whiter.write(&[1], ColumnSet::attr(col)).each(|m| {
        m.attr(col);
    });
    assert!(whiter.same_content(&s), "white is the empty value");
}
