# SPDX-License-Identifier: GPL-3.0-or-later
"""Start to finish through the socket, as the add-on's buttons do it: get the session's parts,
match the scene units, add a UV map in Blender and get the parts again (same points and faces,
so the UV map stays), move one point, and send the parts back.

    blender -b ... --python socket_trip.py -- <socket> <session>

The exchange folders are under `$XDG_CACHE_HOME`, which the test points at its scratch folder.
"""

import os
import sys

sys.dont_write_bytecode = True  # no __pycache__ in the source tree
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import support  # noqa: E402

import bpy  # noqa: E402
import numpy as np  # noqa: E402

from gigasculpt_bridge import bridge, client, folder, meshes  # noqa: E402

socket, session = support.args(2)
support.empty_scene()
with client.Client(socket, "blender-test") as c:
    first = bridge.receive(c, session)
    unit_mm = bridge.match_units(c, bpy.context.scene)
    ob = first[0][0]
    ob.data.uv_layers.new(name="Made in Blender")
    again = bridge.receive(c, session)
    co = meshes.positions(ob.data)
    co[0, 2] += np.float32(0.125)
    ob.data.vertices.foreach_set("co", co.ravel())
    ob.data.update()
    read = bridge.send(c, session, [ob])
support.result({
    "first": [how for _, how in first],
    "again": [how for _, how in again],
    "same_object": again[0][0] == ob,
    "uv_maps": [uv.name for uv in ob.data.uv_layers],
    "unit_mm": unit_mm,
    "scale_length": bpy.context.scene.unit_settings.scale_length,
    "left_in_to_blender": sorted(os.listdir(folder.side(session, "to-blender"))),
    "read": read,
})
