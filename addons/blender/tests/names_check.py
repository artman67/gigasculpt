# SPDX-License-Identifier: GPL-3.0-or-later
"""The name table against Blender itself (decision 0032, section 12, case f).

The manifest GigaSculpt wrote for a painted part must list every row of the add-on's table that
GigaSculpt writes under the same names, domain and type (the add-on refuses it otherwise), and
each Blender name must be what Blender's own helpers write; the color calls the add-on makes
must exist. Face sets have no helper, so for them Blender's program must hold the name.
A rename in a new Blender fails one line here instead of losing data.

    blender -b ... --python names_check.py -- <to-blender>
"""

import os
import sys

sys.dont_write_bytecode = True  # no __pycache__ in the source tree
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import support  # noqa: E402

import bpy  # noqa: E402

from gigasculpt_bridge import folder, names  # noqa: E402

(to,) = support.args(1)
manifest = folder.read_manifest(to)
listed = {names.row(c) for entry in manifest["parts"] for c in entry["columns"]}
# GigaSculpt writes one color row; the other three are what the add-on sends back.
sent = (set(names.TABLE) - set(names.COLORS.values())) | {names.COLOR}

me = bpy.data.meshes.new("names")
me.vertices.add(4)
me.loops.add(4)
me.polygons.add(1)
me.vertices.foreach_set("co", [0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0])
me.loops.foreach_set("vertex_index", [0, 1, 2, 3])
me.polygons.foreach_set("loop_start", [0])
me.update(calc_edges=True)


def made(fn):
    """The one attribute Blender's helper `fn` creates, as (name, domain, type)."""
    before = {a.name for a in me.attributes}
    fn()
    new = [(a.name, a.domain, a.data_type) for a in me.attributes if a.name not in before]
    return new[0] if len(new) == 1 else tuple(new)


def present(name):
    a = me.attributes.get(name)
    return a and (a.name, a.domain, a.data_type)


def table(c):
    kind, domain = names.ATTRIBUTE[c]
    return (c.blender, domain, kind)


with open(bpy.app.binary_path, "rb") as f:
    program = f.read()
loop_start = me.polygons[0].bl_rna.properties.get("loop_start")
color_api = {p.identifier for p in me.color_attributes.bl_rna.properties}
used = {"active_color", "active_color_index", "render_color_index"}


color_values = set()


def new_color():
    """Makes `Color` as the add-on does, and notes what its values offer."""
    a = me.color_attributes.new(names.COLOR.blender, "BYTE_COLOR", "POINT")
    color_values.update(p.identifier for p in a.data[0].bl_rna.properties)


checks = [
    ("every table row in the engine's manifest", listed == sent, sorted((c.blender, c.domain, c.type) for c in listed)),
    ("color", made(new_color) == ("Color", "POINT", "BYTE_COLOR"), present("Color")),
    ("color values", {"color", "color_srgb"} <= color_values, sorted(color_values)),
    ("color api", used <= color_api, sorted(color_api)),
    ("position", present("position") == ("position", "POINT", "FLOAT_VECTOR"), present("position")),
    ("corner_vert", present(".corner_vert") == (".corner_vert", "CORNER", "INT"), present(".corner_vert")),
    ("loop_start", bool(loop_start) and loop_start.type == "INT", loop_start and loop_start.type),
    ("mask", made(me.vertex_paint_mask_ensure) == table(names.MASK), present(".sculpt_mask")),
    ("hide", made(lambda: setattr(me.polygons[0], "hide", True)) == table(names.HIDE), present(".hide_poly")),
    ("hide_vert", made(lambda: setattr(me.vertices[0], "hide", True)) == (names.HIDE_VERT, "POINT", "BOOLEAN"), present(".hide_vert")),
    ("hide_edge", made(lambda: setattr(me.edges[0], "hide", True)) == (names.HIDE_EDGE, "EDGE", "BOOLEAN"), present(".hide_edge")),
    ("crease", made(me.edge_creases_ensure) == (names.CREASE, "EDGE", "FLOAT"), present("crease_edge")),
    ("group", names.GROUP.blender.encode() + b"\0" in program, names.GROUP.blender),
]
fails = [f"{name}: Blender has {seen}" for name, ok, seen in checks if not ok]
support.result({"blender": bpy.app.version_string, "checked": len(checks), "fails": fails})
assert not fails, "; ".join(fails)
