# SPDX-License-Identifier: GPL-3.0-or-later
"""The two directions of the exchange (decision 0032). `receive` asks GigaSculpt to write parts
into `to-blender/` and loads them; `send` writes Blender objects into `from-blender/` and asks
GigaSculpt to read them back, as one undo step there. The socket carries the calls, the folder
carries the data.
"""

import bpy

from . import folder, levels, meshes, names


def load_folder(path, collection=None):
    """Loads every part of an exchange folder. Returns the manifest and (object, how) per part,
    `how` being `in_place`, `rebuilt` or `new`. Every column file is read and checked before
    anything in Blender changes."""
    collection = collection or bpy.context.scene.collection
    manifest = folder.read_manifest(path)
    parts = [(entry, folder.columns(path, entry)) for entry in manifest["parts"]]
    out = []
    for entry, cols in parts:
        ob, how = meshes.load(entry, cols, collection)
        if entry["form"] == "levels" and entry["levels"] > 1:
            meshes.show(ob, True)
            levels.build(ob, entry["levels"], entry.get("creases", []))
        meshes.show(ob, not entry.get("hidden", False))
        out.append((ob, how))
    return manifest, out


def write_folder(objects, path):
    """Writes mesh objects into an exchange folder, column files first and the manifest last.
    An object with a Multires modifier goes in the Levels form: its evaluated top, points and
    faces only, with its level count. Any other goes in the Mesh form with its sculpt columns
    and its active color."""
    # World matrices lag behind a script's edits until the view layer is updated.
    bpy.context.view_layer.update()
    entries = []
    for k, ob in enumerate(objects):
        mod = levels.multires(ob)
        if mod is not None:
            points, corners, starts = levels.evaluated_top(ob, mod)
            sculpt, form, count = {}, "levels", mod.total_levels + 1
        else:
            points = meshes.positions(ob.data)
            corners, starts = meshes.topology(ob.data)
            sculpt, form, count = meshes.sculpt_columns(ob.data), "mesh", 1
        transform, bake = meshes.transform_of(ob)
        if bake is not None:
            # A corner color follows its corners when a mirror reverses them.
            points, order = meshes.baked(points, corners, starts, bake)
            corners = corners[order]
            sculpt = {c: v[order] if c.domain == "corner" else v for c, v in sculpt.items()}
        columns = [
            folder.put_column(path, k, names.POSITION, points),
            folder.put_column(path, k, names.CORNER_VERT, corners),
            folder.put_column(path, k, names.LOOP_START, starts),
        ]
        columns += [folder.put_column(path, k, c, v) for c, v in sculpt.items()]
        part = ob.get(names.PART)
        entries.append({
            "id": None if part is None else str(part),
            "name": ob.name,
            "form": form,
            "levels": count,
            "transform": transform,
            "points": len(points),
            "faces": len(starts),
            "corners": len(corners),
            "columns": columns,
        })
    folder.put_manifest(path, entries)
    return entries


def receive(client, session, form="mesh", parts=None, collection=None):
    """Gets parts from the session (all of them by default) and loads them. The folder's files
    are removed once loaded. Returns (object, how) per part."""
    path = folder.side(session, "to-blender")
    folder.clear(path)
    args = {"folder": path, "form": form}
    if parts is not None:
        args["parts"] = parts
    client.run("exchange.write", args)
    manifest, loaded = load_folder(path, collection)
    folder.remove_listed(path, manifest)
    return loaded


def send(client, session, objects):
    """Sends mesh objects back to the session, which reads them as one undo step. Returns
    `exchange.read`'s result: what became of each object, with a note when it became a new
    part."""
    path = folder.side(session, "from-blender")
    folder.clear(path)
    write_folder(objects, path)
    return client.run("exchange.read", {"folder": path})


def match_units(client, scene):
    """Sets the scene's unit scale so one Blender unit reads as the session's unit, `unit_mm`
    millimetres (decision 0032, section 9). Only when asked: it changes how Blender shows
    lengths, and what it does to Cycles is not checked."""
    unit_mm = client.run("session.state", {})["view.subsurface.unit_mm"]
    scene.unit_settings.scale_length = unit_mm / 1000.0
    return unit_mm
