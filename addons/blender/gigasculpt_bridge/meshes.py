# SPDX-License-Identifier: GPL-3.0-or-later
"""Blender meshes from and to exchange columns, with the data API only (decision 0032).

Every value is copied with `foreach_set` and `foreach_get` through numpy, so it arrives bit for
bit. GoB's in-place rule: an object found by its `gigasculpt_part` property whose point count
and corner list match what arrives only gets new positions, sculpt attributes and colors, so
whatever Blender added (modifiers, UV maps, vertex groups, materials) stays. Otherwise its mesh
is rebuilt in place, which keeps the object, its modifiers and its material slots.
"""

import math

import bpy
import numpy as np
from mathutils import Matrix, Quaternion, Vector

from . import names

IDENTITY = {"location": [0.0, 0.0, 0.0], "rotation": [1.0, 0.0, 0.0, 0.0], "scale": 1.0}


def find(part_id):
    """The first mesh object carrying part id `part_id`, or None."""
    if part_id is None:
        return None
    for ob in bpy.data.objects:
        if ob.type == "MESH" and ob.get(names.PART) == part_id:
            return ob
    return None


def read(attribute, dtype):
    """An attribute's values as a numpy array."""
    out = np.empty(len(attribute.data), dtype)
    attribute.data.foreach_get("value", out)
    return out


def positions(me):
    """A mesh's points, (count, 3)."""
    out = np.empty(len(me.vertices) * 3, np.float32)
    me.vertices.foreach_get("co", out)
    return out.reshape(-1, 3)


def topology(me):
    """A mesh's corner list and face starts."""
    corners = np.empty(len(me.loops), np.int32)
    me.loops.foreach_get("vertex_index", corners)
    starts = np.empty(len(me.polygons), np.int32)
    me.polygons.foreach_get("loop_start", starts)
    return corners, starts


def sizes(corners, starts):
    """Each face's corner count."""
    return np.diff(np.append(starts, len(corners)))


def fill(me, points, corners, starts):
    """Rebuilds `me` with these points and faces. Its materials stay; its other attributes go,
    since they belonged to the old points and faces."""
    me.clear_geometry()
    me.vertices.add(len(points))
    me.loops.add(len(corners))
    me.polygons.add(len(starts))
    me.vertices.foreach_set("co", np.ascontiguousarray(points, np.float32).ravel())
    me.loops.foreach_set("vertex_index", np.ascontiguousarray(corners, np.int32))
    me.polygons.foreach_set("loop_start", np.ascontiguousarray(starts, np.int32))
    me.update(calc_edges=True)


def load(entry, cols, collection):
    """Puts one manifest entry into Blender. Returns the object and what happened to it:
    `in_place` (same points and faces, values written), `rebuilt` (its mesh rebuilt) or `new`."""
    points = cols[names.POSITION.blender]
    corners = cols[names.CORNER_VERT.blender]
    starts = cols[names.LOOP_START.blender]
    ob = find(entry.get("id"))
    if ob is None:
        me = bpy.data.meshes.new(entry["name"])
        fill(me, points, corners, starts)
        ob = bpy.data.objects.new(entry["name"], me)
        collection.objects.link(ob)
        if entry.get("id") is not None:
            ob[names.PART] = entry["id"]
        how = "new"
    else:
        me = ob.data
        # The Mesh form is the whole top: a Multires left on it would divide the top again.
        for m in [m for m in ob.modifiers if m.type == "MULTIRES"]:
            ob.modifiers.remove(m)
        had = topology(me)
        same = len(me.vertices) == len(points) and all(map(np.array_equal, had, (corners, starts)))
        if same and entry["form"] == "mesh":
            me.vertices.foreach_set("co", np.ascontiguousarray(points, np.float32).ravel())
            how = "in_place"
        else:
            fill(me, points, corners, starts)
            how = "rebuilt"
    put_sculpt(me, cols)
    put_color(me, cols.get(names.COLOR.blender))
    place(ob, entry["transform"])
    ob[names.RANK] = min(int(entry.get("rank", 0)), 2**31 - 1)
    ob[names.PAINT] = 0.0 if entry.get("paint_hidden", False) else 1.0
    me.update()
    return ob, how


def put_sculpt(me, cols):
    """Writes the sculpt columns the entry has and removes the ones it has not (GigaSculpt is
    the truth for what it sends), then derives what Blender keeps of hidden faces."""
    for c in names.SCULPT:
        values = cols.get(c.blender)
        kind, domain = names.ATTRIBUTE[c]
        old = me.attributes.get(c.blender)
        if old is not None and (values is None or (old.data_type, old.domain) != (kind, domain)):
            me.attributes.remove(old)
        if values is None:
            continue
        attribute = me.attributes.get(c.blender)
        if attribute is None:
            attribute = me.attributes.new(c.blender, kind, domain)
        attribute.data.foreach_set("value", values.astype(bool) if kind == "BOOLEAN" else values)
    derive_hidden(me)


def put_color(me, values):
    """Writes GigaSculpt's colors, (points, 4) sRGB bytes, as the point byte color `Color` and
    makes it the active and the render color, or removes `Color` when none came. Each byte goes
    in as byte / 255 through `color_srgb`, which Blender keeps as that byte exactly."""
    old = me.attributes.get(names.COLOR.blender)
    if old is not None and (values is None or (old.domain, old.data_type) != ("POINT", "BYTE_COLOR")):
        me.attributes.remove(old)
    if values is None:
        return
    colors = me.color_attributes
    attribute = colors.get(names.COLOR.blender)
    if attribute is None:
        attribute = colors.new(names.COLOR.blender, "BYTE_COLOR", "POINT")
    attribute.data.foreach_set("color_srgb", (values / 255.0).astype(np.float32).ravel())
    colors.active_color = attribute
    colors.render_color_index = colors.active_color_index


def color_column(me):
    """The active color attribute as Blender keeps it, {table row: (count, 4) values}, or {}
    when there is none: bytes as their stored values (`color_srgb` gives each byte back
    exactly), floats as stored (`color`, linear and premultiplied). The engine makes them
    GigaSculpt's colors, so nothing here does color arithmetic."""
    a = me.color_attributes.active_color
    c = None if a is None else names.COLORS.get((a.domain, a.data_type))
    if c is None:
        return {}
    values = np.empty(len(a.data) * 4, np.float32)
    if c.type == "u8":
        a.data.foreach_get("color_srgb", values)
        values = np.rint(values * 255.0).astype(np.uint8)
    else:
        a.data.foreach_get("color", values)
    return {c: values.reshape(-1, 4)}


def derive_hidden(me):
    """`.hide_vert` and `.hide_edge` from `.hide_poly` by decision 0015's rule: hidden when
    there is a face around and every face around is hidden."""
    for name in (names.HIDE_VERT, names.HIDE_EDGE):
        old = me.attributes.get(name)
        if old is not None:
            me.attributes.remove(old)
    poly = me.attributes.get(names.HIDE.blender)
    if poly is None:
        return
    corners, starts = topology(me)
    shown = ~np.repeat(read(poly, bool), sizes(corners, starts))
    edges = np.empty(len(me.loops), np.int32)
    me.loops.foreach_get("edge_index", edges)
    for name, domain, at, count in (
        (names.HIDE_VERT, "POINT", corners, len(me.vertices)),
        (names.HIDE_EDGE, "EDGE", edges, len(me.edges)),
    ):
        around = np.bincount(at, minlength=count)
        showing = np.bincount(at, weights=shown, minlength=count)
        hidden = (around > 0) & (showing == 0)
        me.attributes.new(name, "BOOLEAN", domain).data.foreach_set("value", hidden)


def sculpt_columns(me):
    """The sculpt columns a plain mesh has under the table's types, and its active color, by
    table row."""
    out = {}
    for c in names.SCULPT:
        attribute = me.attributes.get(c.blender)
        if attribute is None or (attribute.data_type, attribute.domain) != names.ATTRIBUTE[c]:
            continue
        if c is names.HIDE:
            out[c] = read(attribute, bool).astype(np.uint8)
        else:
            out[c] = read(attribute, names.DTYPE[c.type])
    out.update(color_column(me))
    return out


def show(ob, visible):
    """Opens or closes the object's eye in the view layer, when it is in it."""
    try:
        ob.hide_set(not visible)
    except RuntimeError:
        pass


def _plain(ob):
    """No parent and no delta transform: the object's own properties are its place."""
    return (
        ob.parent is None
        and tuple(ob.delta_location) == (0.0, 0.0, 0.0)
        and tuple(ob.delta_rotation_quaternion) == (1.0, 0.0, 0.0, 0.0)
        and tuple(ob.delta_rotation_euler) == (0.0, 0.0, 0.0)
        and tuple(ob.delta_scale) == (1.0, 1.0, 1.0)
    )


def place(ob, t):
    """Puts the object at manifest transform `t`, world space. Through its own location,
    rotation and scale when it has no parent or delta transform, so each number arrives
    exactly; else through its world matrix."""
    ob.rotation_mode = "QUATERNION"
    s = t["scale"]
    if _plain(ob):
        ob.location = t["location"]
        ob.rotation_quaternion = t["rotation"]
        ob.scale = (s, s, s)
    else:
        loc, rot = Vector(t["location"]), Quaternion(t["rotation"])
        ob.matrix_world = Matrix.LocRotScale(loc, rot, Vector((s, s, s)))


def _unit(q):
    """A rotation as (w, x, y, z), made unit length only if it is not already close to it, so
    the numbers GigaSculpt sent come back unchanged."""
    q = [float(v) for v in q]
    norm = math.sqrt(sum(v * v for v in q))
    return q if abs(norm - 1.0) <= 1e-6 else [v / norm for v in q]


def transform_of(ob):
    """The object's place as the manifest gives it, and the world matrix to bake into its
    points when that is not a move, a turn and one positive scale (decision 0029), else None."""
    sx, sy, sz = ob.scale
    if _plain(ob) and sx == sy == sz and sx > 0:
        if ob.rotation_mode == "QUATERNION":
            q = ob.rotation_quaternion
        elif ob.rotation_mode == "AXIS_ANGLE":
            angle, *axis = ob.rotation_axis_angle
            q = Quaternion(Vector(axis), angle)
        else:
            q = ob.rotation_euler.to_quaternion()
        location = [float(v) for v in ob.location]
        return {"location": location, "rotation": _unit(q), "scale": sx}, None
    m = ob.matrix_world.copy()
    loc, q, scale = m.decompose()
    s = sum(scale) / 3.0
    similar = Matrix.LocRotScale(loc, q, Vector((s, s, s)))
    off = max(abs(a - b) for row, other in zip(m, similar) for a, b in zip(row, other))
    if m.determinant() > 0 and off <= 1e-6 * max(1.0, max(abs(v) for row in m for v in row)):
        return {"location": [float(v) for v in loc], "rotation": _unit(q), "scale": s}, None
    return dict(IDENTITY), m


def baked(points, corners, starts, m):
    """Points with world matrix `m` baked in, and the corners' new order: each face's corners
    reversed when `m` mirrors, so faces still point outwards. Corner k then is corner order[k],
    for the corner list and every corner column alike."""
    m = np.array(m, np.float64)
    points = (points.astype(np.float64) @ m[:3, :3].T + m[:3, 3]).astype(np.float32)
    order = np.arange(len(corners))
    if np.linalg.det(m[:3, :3]) < 0:
        count = sizes(corners, starts)
        first = np.repeat(starts, count)
        n = np.repeat(count, count)
        order = first + (n - (order - first)) % n
    return points, order
