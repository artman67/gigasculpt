# Brush settings

The brush tray (on the left for a right-handed pen) holds everything about the brush you are
using: its name and its `···` menu at the top, then the sections Basics, Tip, Stroke, Alpha,
Paint and Depth. Click a section's title to fold it.

Size is shared by every brush. Everything else belongs to each brush, so changing Clay's
strength leaves Draw's alone. Your changes last while GigaSculpt runs. To keep a brush the way you set it, save it to a file (see
[Saving and loading brushes](#saving-and-loading-brushes)).

Rest the pointer on any control to see what it does and its key. [Sliders](interface.md#sliders)
says how to drag, type and reset a slider.

## Basics

- **Size** is how big the brush is, in screen pixels, so it stays the same size on screen as you
  zoom. Every brush uses the same size.
- **Strength** is how much each dab changes the surface.
- **Add** and **Subtract** choose which way the brush works: Add builds up, Subtract carves in.
  Holding Alt flips it while you sculpt.
- The pen toggles at the right of Size and Strength choose whether pen pressure changes them.
  Lit, a lighter press makes the brush smaller (Size) or do less (Strength). Most brushes ship
  with Strength's lit and Size's off.
- **Pressure…** opens the brush's own pressure curves; see [Pen pressure](#pen-pressure).

## Softness and the tip

**Softness** is how quickly the brush's effect fades toward its edge. Below 0 gives a soft,
wide edge; above 0 a hard, flat center. With Settings > Interface > Shift snaps Softness on,
Shift held on the slider jumps it to -100, 0 or 100.

The Tip section shows the tip's shape as a small graph, from the center on the left to the rim
on the right. Tap it, or Edit shape…, to open the shape editor:

- **Profiles**: Constant, Hard, Sphere, Flat top, Smooth, Soft, Linear, Sharp and Sharp center,
  from the fullest to the sharpest.
- **The curve**: drag its points to draw your own shape. Reset curve puts it back.
- **Invert** turns the shape upside down, and **Copy** and **Paste** carry it to another brush.
- **Steps** cuts the shape into flat terraces (1 is off), and **Step strength** says how far
  they pull it. **Tile** repeats the shape from the center to the rim (1 is off), for rings.
- **Wrap Mode** repeats each stroke in tiles along each side of the part, 1 (off) to 16.
- **Curve By Pen** lets pressure blend between two shapes: the Zero Curve at no pressure and the
  tip's shape at full, with the Pen Curve saying how much of the shape each pressure takes.

**Exact** reads your own shape exactly instead of from a table: finer detail, a little slower.

## Stroke

How dabs are laid down along a drag:

| Stroke | Short name | What it does |
|---|---|---|
| Dots | Dots | A dab every few pixels along the drag. |
| Drag Rect | Rect | One stamp that you size and turn by dragging. Hold Space to move it. |
| Freehand | Free | Follows the pen with no gaps. |
| Drag Dot | Slide | One stamp that you slide into place. |

- **Spacing** is the gap between dabs, as a share of the size. Lower is smoother and slower.
- **Stabilize** makes the brush trail the pen on a string this long, so lines come out steady;
  0 is off. **Steady** says how firmly the string holds the brush back.

The Move brushes grab and drag, and Clip Curve and Select Rect draw a shape, so these settings
do not change them.

## Alpha

An alpha is a grayscale picture stamped by the tip, for texture and patterned detail.

- The large picture is the brush's alpha. Tap it to open the Library's Alphas row and pick one.
  The alphas you used lately sit beside it as small pictures, one tap each.
- **None** goes back to a plain round tip.
- **Turn** turns the alpha, in degrees, and **Follow** turns it with the stroke's direction (0
  keeps its turn).
- `···` holds **Hold turn**, which keeps the alpha's turn fixed on screen as the surface curves,
  and **Map detail**, which samples the alpha more coarsely for a large brush: 0 keeps every
  detail, higher is faster.

GigaSculpt ships seven alphas: Soft Round, Hard Round, Grainy Round, Square, Ring, Checker and
White. Loading your own picture as an alpha is not built yet.

## Paint

The Paint section holds the colors a brush paints with, and the Color switch that lets a
sculpting brush paint as it sculpts. [Looks and paint](looks.md#painting) covers it.

## Depth

The section opens with **Brush Placement**, a small picture of where the brush sits against the
surface, which follows the settings below.

- **Imbed** moves the brush into the surface (above 0) or out of it (below 0), in brush radii.
  Clay Build and Clay Line ship with a little imbed.
- **Depth Mask** limits the brush to a shell between **Inner** and **Outer**, so it cannot cut
  deeper or build higher than that. While it is on, a Depth Curve under Outer shapes how strongly
  the brush works from the surface out to the shell's edges.
- **Infinite Depth** lets the brush reach through the whole model, along the view or along the
  X, Y and Z buttons that show beside its switch.
- **Gravity** pulls each dab toward a direction (Down, Up, Left, Right, Back or Front); 0 is off.
  Use it for sagging folds, drips and heavy flesh.

## Pen pressure

Pen pressure is set in three places:

1. **The pen toggles** at Size and Strength turn pressure on or off for this brush.
2. **Pressure…** in Basics holds this brush's own curves for Size, Strength, Spacing, the Brush
   Modifier and Depth. Each graph runs from a light touch on the left to full pressure on the
   right. Its switch turns that curve on; a curve that is on starts as a straight line.
3. **Settings > Pen** holds the size curve that every brush uses unless it has its own, Lazy
   pressure (which eases a sudden press or lift), and how much pressure changes size, strength
   and color for every brush. See [Settings](settings.md#pen).

The Move brushes and the Pull Out brushes have flat curves of their own, so pressure leaves
them alone. If pressure does nothing at all, see
[Troubleshooting](troubleshooting.md#pen-pressure-does-nothing).

## Saving and loading brushes

The `···` beside the brush's name holds:

- **Save brush…** writes this brush, with every change you made to it, to a brush file. Your
  system's file browser asks where.
- **Load brush…** reads a brush file back. The file names the shipped brush it is based on;
  that brush becomes the one you use, with the file's settings.
- **Reset to defaults** puts every setting of this brush back as it ships.
- **Copy settings to**: a button for each other brush; click one to give it every setting you
  changed on this one. You keep sculpting with this brush.

A brush file is a text file. It is always a version of one of the 24 shipped brushes and keeps
that brush's name, family and sculpting method, so loading it changes that brush rather than
adding a new one. See [Files](files.md#brush-files).

## Settings with no control yet

A few settings have no control in the brush tray yet: the Brush Modifier (which makes Draw and
the other push brushes pinch toward the stroke or swell out), and auto masking (which keeps a
brush off faces turned away from you, other face groups, and the like). A brush file or the
`brush.set` command changes them; see [Scripts and agents](scripting.md).
