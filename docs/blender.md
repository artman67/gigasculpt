# The Blender add-on

The GigaSculpt Bridge add-on moves parts between a running GigaSculpt and Blender, and back
again. Unlike a mesh file, it keeps the mask, the face groups and the hidden faces, and it
carries the paint as a color attribute. Use it to sculpt in GigaSculpt and then retopologize,
UV, rig or render in Blender, or to bring a Blender mesh in for sculpting.

It needs Blender 5.2 or later.

**Where it works:** on Linux, Windows (10 version 1803 or later) and macOS.

## Installing it

1. Download `gigasculpt-blender-addon-VERSION.zip` from the same release as GigaSculpt.
2. In Blender, choose Edit > Preferences > Add-ons, open the menu at the top right and choose
   Install from Disk, then pick the zip.
3. Turn on **GigaSculpt Bridge** if Blender has not.

The add-on adds a **GigaSculpt** tab to the 3D Viewport's sidebar (press N to show the sidebar).

## Using it

Start GigaSculpt first: the add-on never starts it, and connects only while one of its buttons
runs. Then, in the GigaSculpt tab:

- **Get from GigaSculpt** loads every part of the scene into Blender as mesh objects. Getting
  them again later updates the objects it made before, in place: when a part's points and faces
  are unchanged, only the shape, mask, groups and colors change, so whatever you added in Blender
  (modifiers, UV maps, materials) stays.
- **Send to GigaSculpt** sends the selected mesh objects to GigaSculpt, as one undo step there.
  An object that came from GigaSculpt goes back into its part, keeping the part's detail levels.
  A new object, or one whose points or faces changed, becomes a new part, and Blender shows a
  warning that says so.

If no GigaSculpt answers, the add-on says so: start GigaSculpt, or check the session name.

The tab's settings (also in the add-on's preferences):

- **Session**: the GigaSculpt session to connect to. Leave it as `main` unless you started
  GigaSculpt with another name (`gigasculpt window --session NAME`).
- **Parts with levels**: how a part with detail levels arrives. **Mesh** brings the top level as
  a plain mesh, with its mask, face groups, hidden faces and paint. **Levels** rebuilds the levels
  as a Multires modifier. An object with a Multires modifier goes back to GigaSculpt as levels.
- **Match scene units**: when getting parts, sets Blender's unit scale so one Blender unit reads
  as GigaSculpt's unit in millimeters.

GigaSculpt's File menu has the same two moves under Blender: **Send to Blender** writes every
part for the add-on to load, and **Read from Blender** brings in what the add-on sent, once
there is something new. You seldom need them, since the add-on's buttons do both.

## What comes across

| In GigaSculpt | In Blender |
|---|---|
| Each part | A mesh object |
| Paint | The active color attribute, `Color` |
| Mask, face groups, hidden faces | Blender's own sculpt mask, face sets and hidden faces |
| Detail levels | The top level, or a Multires modifier (see Parts with levels) |

The data travels through an exchange folder in GigaSculpt's cache folder (see
[Files](files.md#where-gigasculpt-keeps-its-files)).
