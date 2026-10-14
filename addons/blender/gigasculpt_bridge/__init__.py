# SPDX-License-Identifier: GPL-3.0-or-later
"""GigaSculpt Bridge: moves parts between a running GigaSculpt session and Blender, keeping the
mask, face groups and hidden faces that every file format drops, and the surface paint as the
active and render color `Color` (decision 0032).

Get from GigaSculpt loads the session's parts; Send to GigaSculpt sends the selected mesh
objects back, where they land as one undo step. The add-on connects to a session by name and
never starts GigaSculpt. Blender 5.2 or newer.
"""

import bpy
from bpy.props import BoolProperty, EnumProperty, StringProperty

from . import bridge, client, folder

bl_info = {
    "name": "GigaSculpt Bridge",
    "version": (0, 1, 0),
    "blender": (5, 2, 0),
    "location": "3D Viewport > Sidebar > GigaSculpt",
    "description": "Move parts between GigaSculpt and Blender, every value kept",
    "category": "Import-Export",
}

MINIMUM = (5, 2, 0)


def _preferences(context):
    return context.preferences.addons[__package__].preferences


class GIGASCULPT_preferences(bpy.types.AddonPreferences):
    bl_idname = __package__

    session: StringProperty(
        name="Session",
        default="main",
        description="The GigaSculpt session to connect to",
    )
    form: EnumProperty(
        name="Parts with levels",
        items=[
            ("mesh", "Mesh", "The top level as a plain mesh, with mask, face groups, hidden faces and paint"),
            ("levels", "Levels", "A Multires modifier rebuilt from the top, with the base creases"),
        ],
        default="mesh",
    )
    match_units: BoolProperty(
        name="Match scene units",
        default=False,
        description=(
            "On getting parts, set the scene's unit scale so one Blender unit reads as "
            "GigaSculpt's unit in millimetres"
        ),
    )

    def draw(self, _context):
        for name in ("session", "form", "match_units"):
            self.layout.prop(self, name)


class GIGASCULPT_OT_receive(bpy.types.Operator):
    """Load every part of the GigaSculpt session, or update the objects it sent before"""

    bl_idname = "gigasculpt.receive"
    bl_label = "Get from GigaSculpt"
    bl_options = {"REGISTER", "UNDO"}

    def execute(self, context):
        prefs = _preferences(context)
        try:
            with client.Client(client.socket_path(prefs.session)) as c:
                loaded = bridge.receive(c, prefs.session, prefs.form)
                if prefs.match_units:
                    bridge.match_units(c, context.scene)
        except (folder.ExchangeError, OSError) as e:
            self.report({"ERROR"}, str(e))
            return {"CANCELLED"}
        self.report({"INFO"}, f"Got {len(loaded)} parts from GigaSculpt")
        return {"FINISHED"}


class GIGASCULPT_OT_send(bpy.types.Operator):
    """Send the selected mesh objects to the GigaSculpt session, as one undo step there"""

    bl_idname = "gigasculpt.send"
    bl_label = "Send to GigaSculpt"

    @classmethod
    def poll(cls, context):
        return any(o.type == "MESH" for o in context.selected_objects)

    def execute(self, context):
        prefs = _preferences(context)
        objects = [o for o in context.selected_objects if o.type == "MESH"]
        try:
            with client.Client(client.socket_path(prefs.session)) as c:
                result = bridge.send(c, prefs.session, objects)
        except (folder.ExchangeError, OSError) as e:
            self.report({"ERROR"}, str(e))
            return {"CANCELLED"}
        for part in result["parts"]:
            if "note" in part:
                self.report({"WARNING"}, f"{part['object']}: {part['note']}")
        self.report({"INFO"}, f"Sent {len(objects)} objects to GigaSculpt")
        return {"FINISHED"}


class GIGASCULPT_PT_bridge(bpy.types.Panel):
    bl_label = "GigaSculpt"
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category = "GigaSculpt"

    def draw(self, context):
        prefs = _preferences(context)
        col = self.layout.column()
        col.prop(prefs, "session")
        col.prop(prefs, "form")
        col.prop(prefs, "match_units")
        col.operator(GIGASCULPT_OT_receive.bl_idname, icon="IMPORT")
        col.operator(GIGASCULPT_OT_send.bl_idname, icon="EXPORT")


CLASSES = (GIGASCULPT_preferences, GIGASCULPT_OT_receive, GIGASCULPT_OT_send, GIGASCULPT_PT_bridge)


def register():
    if bpy.app.version < MINIMUM:
        raise RuntimeError(
            f"GigaSculpt Bridge needs Blender 5.2 or newer; this is Blender "
            f"{bpy.app.version_string}"
        )
    for cls in CLASSES:
        bpy.utils.register_class(cls)


def unregister():
    for cls in reversed(CLASSES):
        bpy.utils.unregister_class(cls)
