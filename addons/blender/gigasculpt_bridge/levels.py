# SPDX-License-Identifier: GPL-3.0-or-later
"""The Levels form (decision 0032, section 2): a part's levels rebuilt in Blender as a Multires
modifier from the top GigaSculpt sent, and the top read back from it.

Unsubdivide (once per level below the top) and Reshape are object-mode operators of the
Multires modifier: the data API has nothing that does their work, and they are the one
exception to the add-on's data-API-only rule. Unsubdivide is Rebuild Subdivisions one level at a
time, so the stack gets exactly the levels GigaSculpt has even when the base could be divided
further.
"""

import contextlib

import bpy
import numpy as np

from . import meshes, names
from .folder import ExchangeError

MODIFIER = "Multires"


def multires(ob):
    """The object's first Multires modifier with levels, or None: such an object goes back in the
    Levels form."""
    for m in ob.modifiers:
        if m.type == "MULTIRES" and m.total_levels > 0:
            return m
    return None


@contextlib.contextmanager
def chosen(*obs):
    """Selects `obs` with the first one active, as the Multires operators want, and puts the
    selection back after. Objects removed meanwhile are left alone."""
    layer = bpy.context.view_layer
    before = [o for o in layer.objects if o.select_get()]
    active = layer.objects.active
    for o in before:
        o.select_set(False)
    for o in obs:
        o.select_set(True)
    layer.objects.active = obs[0]
    try:
        yield
    finally:
        for o in obs:
            o.select_set(False)
        for o in before:
            o.select_set(True)
        layer.objects.active = active


def build(ob, levels, creases):
    """Rebuilds `ob`'s `levels` levels from the top its mesh holds, with GigaSculpt's Multires
    settings, then puts the base creases (both ends as base point indices, and Blender's value)
    on the base and reshapes the top back to what was sent, since creases move the limit
    surface. Any Multires the object had is replaced."""
    for m in [m for m in ob.modifiers if m.type == "MULTIRES"]:
        ob.modifiers.remove(m)
    mod = ob.modifiers.new(MODIFIER, "MULTIRES")
    ob.modifiers.move(len(ob.modifiers) - 1, 0)
    mod.boundary_smooth = "ALL"
    mod.uv_smooth = "NONE"
    mod.use_creases = True
    with chosen(ob):
        for _ in range(levels - 1):
            try:
                bpy.ops.object.multires_unsubdivide(modifier=mod.name)
            except RuntimeError as e:
                raise ExchangeError(
                    f"Blender could not rebuild level {mod.total_levels + 1} of {levels} of "
                    f"'{ob.name}' from its top: {e}"
                ) from None
    if mod.total_levels != levels - 1:
        raise ExchangeError(
            f"Blender rebuilt {mod.total_levels + 1} levels of '{ob.name}', {levels} were sent"
        )
    mod.levels = mod.sculpt_levels = mod.render_levels = mod.total_levels
    if not creases:
        return
    points, corners, starts = evaluated_top(ob, mod)
    put_creases(ob.data, creases)
    temp = bpy.data.objects.new("gigasculpt top", bpy.data.meshes.new("gigasculpt top"))
    meshes.fill(temp.data, points, corners, starts)
    bpy.context.scene.collection.objects.link(temp)
    try:
        with chosen(ob, temp):
            bpy.ops.object.multires_reshape(modifier=mod.name)
    finally:
        top = temp.data
        bpy.data.objects.remove(temp)
        bpy.data.meshes.remove(top)


def put_creases(me, creases):
    """Sets `crease_edge` on the base edges GigaSculpt creased, and zero on the rest."""
    edges = np.empty(len(me.edges) * 2, np.int32)
    me.edges.foreach_get("vertices", edges)
    index = {frozenset(e): i for i, e in enumerate(edges.reshape(-1, 2).tolist())}
    values = np.zeros(len(me.edges), np.float32)
    for a, b, value in creases:
        e = index.get(frozenset((a, b)))
        if e is None:
            raise ExchangeError(f"the crease from point {a} to {b} is not an edge of the base")
        values[e] = value
    attribute = me.attributes.get(names.CREASE)
    if attribute is None:
        attribute = me.attributes.new(names.CREASE, "FLOAT", "EDGE")
    attribute.data.foreach_set("value", values)


def evaluated_top(ob, mod):
    """The points, corner list and face starts of `ob`'s top level, in Blender's order, with
    only `mod` evaluated. Every setting it touches is put back."""
    saved = [(m, m.show_viewport) for m in ob.modifiers]
    level, hidden = mod.levels, ob.hide_get()
    for m, _ in saved:
        m.show_viewport = m == mod
    mod.levels = mod.total_levels
    if hidden:
        ob.hide_set(False)
    try:
        graph = bpy.context.evaluated_depsgraph_get()
        graph.update()
        evaluated = ob.evaluated_get(graph)
        me = evaluated.to_mesh()
        try:
            return (meshes.positions(me), *meshes.topology(me))
        finally:
            evaluated.to_mesh_clear()
    finally:
        for m, shown in saved:
            m.show_viewport = shown
        mod.levels = level
        if hidden:
            ob.hide_set(True)
