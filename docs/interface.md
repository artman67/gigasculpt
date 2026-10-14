# The interface

GigaSculpt's window is the Workbench: the 3D view in the middle, with shelves and trays around
it grouped by task. Every button runs a named command, the same one `gs` and scripts run, and
hovering over a control says what it does and its key.

![The Workbench](images/workbench.png)

## The parts of the window

- **The header** runs along the top of the 3D view with the menus: File, Add, Part, Mesh, Mask,
  Visible, Groups, Deform, View and Window. Its right end holds the view's buttons: Frame,
  Isolate (show the active part alone), Look (shading, MatCap, lighting and the material's
  sliders) and Symmetry (on and off, X, Y, Z, and under its arrow the mirror, parallel and
  radial kinds per axis).
- **The brush shelf** holds the Gizmo tool, then a button per brush family: Build, Carve,
  Move, Smooth, Mask and Paint. A tap picks the family's brush you used last; a second tap, a
  held press or a right click opens the whole family. Favorites pinned in the Library follow,
  then Quick.
- **The brush tray** (on the left for a right-handed pen) holds what the next stroke does: the
  brush's name and its `···` menu (Save brush, Load brush, Reset to defaults), then Basics
  (size, strength and the pen's pressure), Tip (the falloff curve and Softness), Stroke (dots,
  rectangle, freehand or sliding strokes, spacing and stabilizing), Alpha, Paint and Depth.
- **The scene tray** (on the right) holds the Parts list with each part's Visible switch, Detail
  (subdivision levels and Live Detail) and History (Undo and Redo, each naming its step, and
  Adjust).
- **The status line** along the bottom says what just happened, and shows a bar with Cancel
  while a long command runs.

![The brush shelf and the brush tray](images/brush-tray.png)

Windows that open on top:

| Opens with | What |
|---|---|
| Shift+Space | The quick menu at the pointer: the brush's Size, Strength and Softness, the symmetry switches, the mask's operations and the history. |
| Space | The command palette: type a few letters to find any command and run it. |
| F9 | The adjust panel: change the settings of the command you just ran, such as a new sphere's density or a remesh's resolution. |
| Comma | The Library: brushes, alphas, materials, environments, layouts and themes. F6 opens it on the materials. |
| F2, held | The brush wheel. |
| Window > Settings | The pen, held keys, navigation, sliders, the interface scale, the theme, files, hole closing and the keys. |

![The quick menu](images/quick-menu.png)

Window > Left hand and Right hand swap the trays' sides. Drag a tray's inner edge to make it
wider or narrower, and click a section's title to fold it. Tab hides the panels and shows them
again. Ctrl+Shift+1, 2 or 3 saves the arrangement on screen, and Ctrl+1, 2 or 3 brings it back.

## Moving the view

| Press and drag | Drag | With Alt | With Ctrl | Shift during |
|---|---|---|---|---|
| Right mouse button, or the pen's barrel button, anywhere | Orbit | Pan | Zoom | Land on the nearest side view |
| Left mouse or pen tip on empty canvas | Orbit | Pan | | Land on the nearest side view |
| Middle mouse button | Pan | | | |

The mouse wheel zooms toward the surface under the pointer. The figure in the view's corner
orbits when dragged, and the Zoom and Pan buttons under it zoom and pan, for a pen with no
buttons.

## Held keys while sculpting

| Hold | The stroke |
|---|---|
| Shift | Smooths, with the Smooth brush |
| Ctrl | Paints a mask, with the Mask Pen |
| Shift+Ctrl | Selects what shows, with Select Rect |
| Alt | Works the other way: digs instead of building, unmasks instead of masking |

Ctrl alone on empty canvas works on the mask: a tap inverts it and a drag clears it.

## Keys

### General

| Key | Does |
|---|---|
| Space | Command palette |
| Shift+Space | Quick menu |
| Ctrl+Z | Undo |
| Ctrl+Shift+Z, Ctrl+Y | Redo |
| Ctrl+S | Save |
| Ctrl+Alt+S | Save under the next number |
| 9 | Quick save to the autosave folder |
| Ctrl+O | Open a scene |
| Ctrl+N | New scene |
| Ctrl+Shift+T | Save the active part as a Part file |
| Ctrl+Alt+R | Revert to the saved file |
| Tab | Hide and show the panels |
| Comma | Library |
| F2 (held) | Brush wheel |
| F6 | Materials |
| F9 | Adjust panel |
| Ctrl+1, 2, 3 | Load arrangement 1, 2, 3 |
| Ctrl+Shift+1, 2, 3 | Save arrangement 1, 2, 3 |

### View

| Key | Does |
|---|---|
| F | Frame every part; press again for the active one |
| Alt+F | Frame the mask |
| Period | Frame the last edit |
| Shift+F | Wireframe on and off |
| P | Perspective on and off |
| Shift+P | Floor on and off |
| V then X, Y or Z | Look along X, Y or Z (again: from the other side) |
| V then I | Iso view |
| V then H | Level the view |
| V then L | Lock the view |
| Alt+Z, Shift+Alt+Z | Undo and redo a view move |

### Brushes

| Key | Does |
|---|---|
| B then two letters | Pick a brush: B S T Draw, B C B Clay Build, B C L Clay Line, B I N Inflate, B T D Trim, B C C Clip Curve, B P I Pinch, B D S Sharp Crease, B M V Move, B M E Move Elastic, B S H Pull Out, B S M Smooth, B S S Smooth Strong, B M P Smooth Polish, B H P Flat Polish |
| S, U, O (held, then drag) | Size, strength, softness |
| [ and ] | Size down and up |
| L | Stroke smoothing on and off |
| Backtick | The brush used before |
| X | Mirror across X on and off |
| C | Pick the color under the pointer |
| Shift+X | Swap the two paint colors |

### Mask and visibility

| Key | Does |
|---|---|
| Ctrl+H | Show the mask on and off |
| Ctrl+I | Invert the mask |
| Ctrl+A | Mask everything |
| Alt+M | Clear the mask |
| Ctrl+W | Turn the mask into a face group and clear it |
| Ctrl+Shift+X, Ctrl+Shift+S | Grow, shrink what shows |
| Ctrl+Shift+A, Ctrl+Shift+Q | Show whole shells, whole groups |

### Geometry

| Key | Does |
|---|---|
| Ctrl+D | Subdivide |
| D, Shift+D | One level up, one level down |
| Ctrl+R | Voxel Remesh |
| Backslash | Live Detail on and off |
| Ctrl+Shift+D | Duplicate the part |

## Changing keys

Hold Ctrl+Alt and click any button or switch, then press the key you want for it (Escape
cancels). If another command holds that key, GigaSculpt asks before replacing it. Settings >
Keys lists every key in force. Your changes are kept in `keymap.toml` in GigaSculpt's config
folder (on Linux `~/.config/gigasculpt`), on top of the built-in keys above.
