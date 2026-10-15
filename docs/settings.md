# Settings

Window > Settings opens the settings window at the top corner of the 3D view. It shows one page
at a time: Pen, Interface, Cursor, Navigation, Files, Geometry and Keys. It stays open while you
work the view, so you can try a navigation speed as you set it.

- **Search settings** at the top finds a setting by its words; click a result to open its page.
- Each setting has its own **Reset**, and the foot of the window has **Reset page**, **Reset all**
  and **Close**. A reset asks before it puts the defaults back.
- Rest the pointer on a setting to see what it does.

GigaSculpt keeps these settings in `preferences.toml` in its config folder, except the autosave
settings, which are in the state folder (see [Files](files.md#where-gigasculpt-keeps-its-files)).
The command palette finds every setting by name too.

## Pen

- **Every brush**: a pressure curve for size that every brush uses unless it has its own (see
  [Brush settings](brush-settings.md#pen-pressure)). The switch turns it on, as a straight line
  to start; drag its points to shape it.
- **Lazy pressure** eases each moment's pressure toward the pen's, so a sudden press or lift comes
  in gently. 0 follows the pen at once.
- **Sensitivity**: **Size**, **Strength** and **Color** set how much pressure changes every
  brush's size, strength and paint. 100 uses the pressure curve as it is, 0 leaves pressure out,
  and below 0 runs it backwards.
- **Eraser**: what the pen's eraser end does. **Tip** strokes as the tip does; **Shift**, **Ctrl**,
  **Alt** and **Ctrl+Alt** stroke as if those keys were held, from its first touch until it leaves
  the tablet's range.
- **Click**: what a click of the pen's barrel button does: open the brush wheel (**Wheel**), the
  command palette (**Palette**) or the quick menu (**Quick**) at the pen, or nothing (**None**). A
  barrel drag always moves the view.
- **Button 2**: what the pen's second barrel button, or the middle mouse button, does: **Pan** the
  view, or act as **Shift**, **Ctrl** or **Alt** while it is down.

## Interface

**Held keys**

- **Sticky**: **Modifiers** makes a tap of Shift, Ctrl or Alt keep it on until the next tap (a long
  hold still lets go with the key). **Slider keys** makes S, U, O or any slider's key, held a
  moment, set that slider as the pointer moves, and a tap show it.
- **Name held keys**: the status line says what the held keys do. The cursor always shows it.
- **Hold (s)**: how long a slider's key is held before the pointer sets the slider.

**Sliders**

- **Show slider values**: every slider shows its value; off, a value shows only while the pointer
  is over it.
- **Stop at the middle**: a drag along a slider that runs through 0 stops there a moment.
- **Shift snaps Softness**: Shift held on Softness takes it to -100, 0 or 100.
- **Step**: **Strength** and **Softness** set how far one step moves them, for a key that steps
  them.

**Brush wheel**

- **Offset**: how far the brush wheel opens from the pen, away from the hand that holds it.

**Interface scale**

- **75%**, **100%**, **125%**, **150%** and **200%** of the size your display asks for.

**Parts list**

- **Rows**: how many parts the list shows before it scrolls, 1 to 24.

**Theme**

- **Dark**, **Light** and **High contrast**, with **Previous** and **Next** to step through them.
  Theme files of your own go in the `themes` folder of the config folder.

## Cursor

- **Rings**: **On surface** lays the brush's Size and Softness rings flat on the surface under the
  pen; off, they stay circles on the screen. **Mirrored** shows quieter rings where each dab
  repeats while symmetry is on (see [Symmetry](symmetry.md)).
- **Gizmo size**: the Gizmo's size on screen (see [Parts, the Gizmo and Deform](parts.md#the-gizmo)).

## Navigation

- **Empty press turns view**: a press on empty canvas turns the view; off, it sculpts as a press on
  the model does.
- **Orbit**: how a drag turns the view. **Y** (as it ships) turns about the scene's up axis like a
  turntable, keeping the horizon level. **X** and **Z** turn about those axes instead, and
  **Free** turns about the screen's own axes, so the view may tip over.
- **Pivot**: what the view turns and zooms about. **Middle** (as it ships) is the middle of the
  picture; **Last edit** is where your last stroke first touched; **Parts** is the center of every
  part; **Pointer** is the surface under the press that starts the move.
- **Recenter**: how far each quarter turn slides the pivot toward the middle of the picture.
- **Snap**: the step a snapping orbit lands on: 5°, 15°, 30°, 45° or 90° (as it ships, the six
  side views).
- **Zoom changes distance**: a zoom brings the camera nearer, so perspective grows up close.
- **Pan by lens**: a pan in perspective slides the picture, so you see the model from the same side
  anywhere on the screen.
- **Right press moves pivot**: a right press on the model moves the last-edit point there.
- **Speed**: **Turn**, **Drag zoom** and **Wheel zoom**.

[The interface](interface.md#moving-the-view) says which drags orbit, pan and zoom.

## Files

- **Open last scene**: at start, opens the newest recent scene, unless there is crashed work to
  offer back.
- **Brush at start**: **Last used** starts with the brush you used last; **Pinned** pins the brush
  in use, so every start begins with it.
- **Autosave**: **Every (min)**, **Idle (min)** and **Copies**. See
  [Autosave and recovery](files.md#autosave-and-recovery).

## Geometry

- **Hole closing**: whether Voxel Remesh, Voxel mode and its updates close a part's holes with
  large triangles first (**Large triangles**, as it ships) or leave them open (**None**). See
  [Voxel Remesh](detail.md#voxel-remesh).

## Keys

The Keys page lists every key in force, under where it works. **Bind a key…** opens the command
palette's list: pick what the key runs, then press the key, and **Bind** binds it. A key that
already runs something asks whether to replace it. Under **Your changes**, each key you changed has
a button to put it back, and **Restore defaults** puts every key back as built.

To bind a key to a button straight from the window, hold Ctrl+Alt and click the button; see
[Changing keys](interface.md#changing-keys). Your keys are kept in `keymap.toml` in the config
folder. [Keys](keys.md) lists every built-in key.
