# SPDX-License-Identifier: GPL-3.0-or-later
"""One trip through Blender: loads what GigaSculpt wrote with the add-on's own code, makes one
edit, and writes the objects back as the add-on sends them. Also writes the first object's
points in world space, Blender's axes, as f64 into `<extras>/world.bin`.

    blender -b ... --python trip.py -- <to-blender> <from-blender> <extras> <edit>

Edits: `none`; `point` (point 0 up by 0.125 in part space); `turn` (the object moved 1 along x
and turned a quarter about z, in world space); `topology` (two faces next to each other
dissolved into one polygon, and a loose point added); `paint` (a FLOAT_COLOR on corners, not
named `Color`, made active: point 0's corners blue at half coverage, every other corner red).

The result gives each object's active color as loaded (name, domain, type and the render
color's name) and its `gigasculpt_paint` property.
"""

import math
import os
import sys

sys.dont_write_bytecode = True  # no __pycache__ in the source tree
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import support  # noqa: E402

import bmesh  # noqa: E402
import numpy as np  # noqa: E402
from mathutils import Quaternion  # noqa: E402

from gigasculpt_bridge import bridge, levels, meshes  # noqa: E402


def point(ob):
    co = meshes.positions(ob.data)
    co[0, 2] += np.float32(0.125)
    ob.data.vertices.foreach_set("co", co.ravel())
    ob.data.update()


def turn(ob):
    ob.location.x += 1.0
    ob.rotation_quaternion = Quaternion((0.0, 0.0, 1.0), math.pi / 2) @ ob.rotation_quaternion


def topology(ob):
    bm = bmesh.new()
    bm.from_mesh(ob.data)
    bm.faces.ensure_lookup_table()
    f = bm.faces[0]
    g = next(x for x in f.edges[0].link_faces if x is not f)
    bmesh.ops.dissolve_faces(bm, faces=[f, g])
    bm.verts.new((5.0, 5.0, 5.0))
    bm.to_mesh(ob.data)
    bm.free()
    ob.data.update()


def paint(ob):
    me = ob.data
    a = me.color_attributes.new("Painted", "FLOAT_COLOR", "CORNER")
    corners, _ = meshes.topology(me)
    colors = np.tile(np.array([1.0, 0.0, 0.0, 1.0], np.float32), (len(corners), 1))
    colors[corners == 0] = (0.0, 0.0, 0.5, 0.5)
    a.data.foreach_set("color", colors.ravel())
    me.color_attributes.active_color = a


def color(ob):
    """The active color as loaded: name, domain, type and the render color's name."""
    ca = ob.data.color_attributes
    a = ca.active_color
    if a is None:
        return None
    return [a.name, a.domain, a.data_type, ca[ca.render_color_index].name]


EDITS = {"none": lambda ob: None, "point": point, "turn": turn, "topology": topology, "paint": paint}

to, back, extras, edit = support.args(4)
support.empty_scene()
_, loaded = bridge.load_folder(to)
objects = [ob for ob, _ in loaded]
colors = [color(ob) for ob in objects]
EDITS[edit](objects[0])
bridge.write_folder(objects, back)
m = np.array(objects[0].matrix_world, np.float64)
world = meshes.positions(objects[0].data).astype(np.float64) @ m[:3, :3].T + m[:3, 3]
world.astype("<f8").tofile(os.path.join(extras, "world.bin"))
support.result({
    "how": [how for _, how in loaded],
    "levels": [mod.total_levels if mod else 0 for mod in map(levels.multires, objects)],
    "polygons": [len(ob.data.polygons) for ob in objects],
    "color": colors,
    "paint": [ob.get("gigasculpt_paint") for ob in objects],
})
