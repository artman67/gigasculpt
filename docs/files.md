# Files

GigaSculpt saves scenes as `.gss` files. This page covers saving and opening, autosave and
recovery, importing and exporting meshes, brush files, and where GigaSculpt keeps its own files.

## Saving and opening

Everything here is in the File menu, which shows a shorter name for some items (Save as for Save
scene as, Open recent for Open a recent scene).

- **New scene** (Ctrl+N) starts an empty scene with no file.
- **Open scene** (Ctrl+O) opens a scene, or a Part file, in place of this scene. Picking a mesh
  file imports it instead.
- **Open a recent scene** lists the last 10 scenes you opened or saved, newest first. A file that
  has been moved or deleted shows dimmed.
- **Save** (Ctrl+S) writes the scene to its own file and keeps the old one beside it as a backup
  (`head.backup.gss` for `head.gss`). A scene that was never saved opens Save scene as instead.
- **Save scene as** (Ctrl+Shift+S) writes the scene to a file you choose, which becomes its own
  file from then on. A name typed without `.gss` gets it: `head` saves `head.gss`. To write an
  OBJ, PLY or STL file, use Export a file.
- **Save next numbered** (Ctrl+Alt+S) writes the scene under the next number after its file's
  name, so `head_001` becomes `head_002`, and that becomes its own file. Use it to keep every
  stage of a sculpt.
- **Quick save** (9) writes a numbered copy into the scene's autosave folder and leaves its own
  file alone.
- **Revert to saved** (Ctrl+Alt+R) opens the scene's own file again, dropping every change since
  you saved. It asks first: the palette opens with a **Revert to saved** button, which you click
  (or press Enter) to go ahead. It is gray until the scene has a file of its own.

Save scene as, Open scene, Save Part file, Import a file and Export a file open your system's
own file browser to ask for the file, and so do Save and Save next numbered on a scene that has
no file yet, and Save next Part file the first time. Where it cannot show, GigaSculpt asks in a
box of its own instead; given a folder with no file name after it, the box stays open and asks
you to type one.

Each save says which file it wrote on the status line, such as `Saved head.gss`, for a few
seconds; autosave does not.

The window's title names the scene's file, with a `*` in front while there are unsaved changes:
`*head.gss - GigaSculpt`. New scene, Open scene (and Open a recent scene) and closing the window
ask before they drop unsaved changes: **Save**, **Don't save** or **Cancel**. Save saves first,
asking for a name if the scene has none, then goes on; Enter answers Save. With nothing unsaved
there is no question.

Two more ways to open a scene:

- Drop a scene file on the window. Dropping a mesh file imports it instead.
- Give its path when you start the app: `gigasculpt head.gss`. A mesh file's path imports it.

Settings > Files > Open last scene opens the newest recent scene each time GigaSculpt starts.

### Part files

A Part file holds one part alone. **Save Part file** writes the active part to one, and **Save
next Part file** writes it under the next number after the Part file you saved last. Both sit
under Part file in the File menu. Open scene opens a Part file as a scene of its own.

### What a scene file keeps

A scene file keeps every part with its detail levels, mask, face groups, hidden faces and paint,
and the scene's looks (with your changes and copies) and its background. Your settings, keys and
layout are not in it; they stay in GigaSculpt's config folder.

## Autosave and recovery

While you work, GigaSculpt writes a copy of the scene into its autosave folder now and then. It
skips the copy when nothing changed since the last one, while the pen is down, and for an empty
scene. Settings > Files sets when:

| Setting | What it does | Ships at |
|---|---|---|
| Every (min) | A copy once this many minutes have passed since the last save of any kind. At the top of the slider (60), never. | 10 |
| Idle (min) | A copy after this many minutes without input. At the top of the slider (30), never. | 2 |
| Copies | How many copies of each scene to keep; a new one replaces the oldest. | 10 |

These three are kept in `autosave.json` in the state folder, not with the other settings.

If GigaSculpt stops unexpectedly, the next start shows **Recover your work**:

- **Recover** opens the newest autosave, then runs again every step you took after it, so the
  scene comes back to your last step, not only to the last autosave. It shows when GigaSculpt can
  do this: the log of those steps must be there and written by the same version.
- **Autosave only** opens the newest autosave alone.
- **Discard** deletes that session's autosaves for good, once you confirm with Delete.
- **Later** closes the question until you pick **File > Recover…**, or until the next start.

If Recover asks first about unsaved changes and you answer Cancel, the offer comes back.

A recovered scene keeps its file's name but counts as unsaved, so save it to keep it.

`gigasculpt window --no-autosave` turns off both autosave and recovery, and a session with
autosave off keeps no recent scenes either.

A second GigaSculpt window that is still open is never offered as crashed work, on any system.

## Importing and exporting

**File > Import a file** reads an OBJ, PLY, STL, ZTL or ZPR file into the scene as new parts, in
one undo step:

- every object in the file becomes a part, named after it (or after the file);
- face groups, UVs, vertex colors and the mask come along when the file holds them;
- polygons with more than four corners become triangles, and normals are worked out again;
- a part from a ZTL or ZPR file keeps its detail levels, with the level it was saved at active.

**File > Export a file** writes one part, or every part, to an OBJ, PLY or STL file, chosen by
the name's extension; a name without one writes OBJ (`out` writes `out.obj`):

| Format | Keeps |
|---|---|
| OBJ | Each part as an object, with face groups, UVs, vertex colors and the mask. |
| PLY | Vertex colors. |
| STL | Triangles only, for 3D printing. |

A part you imported remembers the size and place it had in its file, so exporting it writes it
back the same way.

To work on a part in Blender and bring it back with its mask, face groups and hidden faces kept,
use [the Blender add-on](blender.md) instead.

## Brush files

A brush file keeps one brush with every setting you changed, in a small text file (`.toml`). In
the brush tray, the `···` beside the brush's name has **Save brush…** and **Load brush…**, and
your system's file browser asks where. See
[Brush settings](brush-settings.md#saving-and-loading-brushes).

A brush file is always a version of one of the 24 shipped brushes: it keeps that brush's name,
family and way of sculpting, so loading it changes that brush rather than adding a new one. Its
alpha is saved by name. Comments you write in the file are kept.

## Where GigaSculpt keeps its files

GigaSculpt keeps three folders of its own:

| Folder | Linux | macOS | Windows |
|---|---|---|---|
| Config | `~/.config/gigasculpt` | `~/Library/Application Support/GigaSculpt` | `%APPDATA%\GigaSculpt` |
| State | `~/.local/state/gigasculpt` | `~/Library/Application Support/GigaSculpt` | `%LOCALAPPDATA%\GigaSculpt` |
| Cache | `~/.cache/gigasculpt` | `~/Library/Caches/GigaSculpt` | `%LOCALAPPDATA%\GigaSculpt\cache` |

On any system, an absolute `XDG_CONFIG_HOME`, `XDG_STATE_HOME` or `XDG_CACHE_HOME` puts that
folder at `gigasculpt` inside it instead. `gigasculpt window --config FOLDER` uses another config
folder for one run.

**Config** holds your settings:

- `preferences.toml`: what the settings window sets;
- `keymap.toml`: your own keys, on top of the built-in ones;
- `layout.toml`: the layout shown, the trays' widths and folded sections, favorites and the
  like;
- `layouts/` and `themes/`: layout and theme files of your own;
- `materials/`: look files of your own (see [Looks and paint](looks.md#material)).

**State** holds what must survive a restart:

- `autosave/`: a folder of autosaves for each scene;
- `autosave.json`: the autosave settings;
- `recent.json`: the recent scenes;
- `logs/`: a log of every command each session ran, one file per session, which a bug report can
  include (see [Troubleshooting](troubleshooting.md#reporting-a-bug)), and beside the log of each
  running window a small `.lock` file it holds, so tidying the folder never deletes a live log;
- `sessions/`: two small files for each running window, which recovery reads: one names its
  autosaves and log, and the window holds the other locked while it runs. On Windows and macOS
  this folder also holds the sockets that `gs` and the Blender add-on connect to; on Linux they
  are in `$XDG_RUNTIME_DIR/gigasculpt`.

**Cache** holds what can be made again: the exchange folders of [the Blender add-on](blender.md)
and the Quad Remesh helper program. Deleting it loses none of your work.
