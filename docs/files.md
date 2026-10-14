# Files

GigaSculpt saves scenes as `.gss` files. This page covers saving and opening, autosave and
recovery, importing and exporting meshes, brush files, and where GigaSculpt keeps its own files.

## Saving and opening

Everything here is in the File menu, which shows a shorter name for some items (Save as for Save
scene as, Open recent for Open a recent scene).

- **New scene** (Ctrl+N) starts an empty scene with no file.
- **Open scene** (Ctrl+O) opens a scene, or a Part file, in place of this scene.
- **Open a recent scene** lists the last 10 scenes you opened or saved, newest first. A file that
  has been moved or deleted shows dimmed.
- **Save** (Ctrl+S) writes the scene to its own file and keeps the old one beside it as a backup
  (`head.backup.gss` for `head.gss`). A scene that was never saved opens Save scene as instead.
- **Save scene as** (Ctrl+Shift+S) writes the scene to a file you choose, which becomes its own
  file from then on.
- **Save next numbered** writes the scene under the next number after its file's name, so
  `head_001` becomes `head_002`, and that becomes its own file. Use it to keep every stage of a
  sculpt.
- **Quick save** writes a numbered copy into the scene's autosave folder and leaves its own file
  alone.
- **Revert to saved** opens the scene's own file again, dropping every change since you saved.

Save scene as, Open scene, Save Part file, Import a file and Export a file open your system's
own file browser to ask for the file.

The window's title names the scene's file and shows when there are unsaved changes. New scene,
Open scene and Revert to saved ask before they drop unsaved changes, and so does closing the
window: Save, Don't save or Cancel.

Two more ways to open a scene:

- Drop a scene file on the window. Dropping a mesh file imports it instead.
- Give its path when you start the app: `gigasculpt head.gss`.

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
- **Discard** deletes that session's autosaves for good.
- **Later** closes the question until the next start.

A recovered scene keeps its file's name but counts as unsaved, so save it to keep it.

`gigasculpt window --no-autosave` turns off both autosave and recovery, and a session with
autosave off keeps no recent scenes either.

Telling a crashed session from one still running works on Linux only for now. On Windows and
macOS, a second GigaSculpt window open at the same time may be offered as crashed work: choose
Later there, not Discard.

## Importing and exporting

**File > Import a file** reads an OBJ, PLY or STL file into the scene as new parts, in one undo
step:

- every object in the file becomes a part, named after it (or after the file);
- face groups, UVs, vertex colors and the mask come along when the file holds them;
- polygons with more than four corners become triangles, and normals are worked out again.

**File > Export a file** writes one part, or every part, to an OBJ, PLY or STL file:

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
  include (see [Troubleshooting](troubleshooting.md#reporting-a-bug));
- `sessions/`: a small file for each running window, which recovery reads. On Windows and macOS
  this folder also holds the sockets that `gs` and the Blender add-on connect to; on Linux they
  are in `$XDG_RUNTIME_DIR/gigasculpt`.

**Cache** holds what can be made again: the exchange folders of [the Blender add-on](blender.md)
and the Quad Remesh helper program. Deleting it loses none of your work.
