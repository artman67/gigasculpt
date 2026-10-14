# Brushes

GigaSculpt ships 24 brushes in six families. Each family is a button on the brush shelf at the
top of the window: a tap picks the brush it shows, and a held press, a right click, or a tap
while it is lit opens the whole family. Rest the pointer on a brush to see its key; [Keys](keys.md)
lists them all.

Every brush is a preset: a few settings on top of one of a handful of sculpting methods. You can
change any brush's settings in the brush tray, save your version and load it later; see
[Brush settings](brush-settings.md).

A few things hold for every family:

- **Alt** works the other way while held: a brush that builds digs, a mask brush unmasks, and a
  paint brush paints the Secondary color.
- **Shift** smooths with the brush the Smooth family shows, and **Ctrl** masks with Mask Pen,
  whatever brush is picked. See [Held keys](interface.md#held-keys-while-sculpting).
- **The mask** holds back the brushes that shape the surface, so masked areas stay still. See
  [Masking](masking.md).
- **Pen pressure** changes how strongly most brushes work. The Move family and the Pull Out
  brushes ignore pressure for size and strength, so a light touch grabs as much as a hard one.

## Build

Brushes that add form.

| Brush | What it does | Reach for it when |
|---|---|---|
| **Draw** | Pushes the surface out along the average direction it faces under the brush. | You want a plain, even build-up or a groove (with Alt). The all-round first brush. |
| **Clay** | Pushes the surface out evenly, made to stamp alphas. It ships with a grainy round tip. | You want texture from an alpha, or a soft, grainy build-up. |
| **Clay Build** | Fills up to a flat plane fitted under each dab, a little above the surface, so passes stack in flat-topped layers. It fills hollows and covers detail. Ships with a square tip. | Blocking in muscles and big planes, the way you lay on strips of clay. |
| **Clay Line** | Clay Build laid along a straight line from where you press to where you let go. | A straight strip of clay, such as an edge, a seam or a brow line. |
| **Inflate** | Pushes each point out along its own direction, so the surface swells like a balloon. With Alt it shrinks. | Swelling a form evenly, such as a lip, a finger or a cheek. |

## Carve

Brushes that cut, sharpen and flatten.

| Brush | What it does | Reach for it when |
|---|---|---|
| **Sharp Crease** | Cuts a narrow line inward and pulls the sides toward it, so the crease stays crisp. It ships with Stabilize on for steady lines. | Wrinkles, folds, eyelids and the line between two forms. |
| **Pinch** | Pulls the surface toward the brush's center, along the surface, so a stroke tightens toward its line. | Sharpening an edge or a ridge that has gone soft. |
| **Trim** | Shaves off what sticks up above a plane fitted at each dab, with a crisp edge. It never fills hollows. | Cutting hard planes and flat facets into a form. |
| **Clip Curve** | Draw a line across the view; everything on its shaded side is cut flat along that line, straight through the whole model. | Cutting a part straight or along a curve in one move, such as the sole of a shoe or a flat base. |

Clip Curve draws a line from where you press. While you drag, tap Alt to put a point down, and
tap it twice quickly to make that point a sharp corner. Hold Space to move the whole line. The
shaded side of the line is the side that is cut; while Alt is down, the other side is. The cut
happens when you let go, and the mask holds it back.

## Move

Brushes that grab and pull. None of them take pressure for size or strength.

| Brush | What it does | Reach for it when |
|---|---|---|
| **Move** | Grabs what is under the brush when you press and drags it with the pointer. Dragging out and back puts it exactly where it was. | Changing big shapes and proportions. |
| **Move Connected** | Like Move, but only grabs what is joined to the point you press along the surface, so it never drags a nearby lip, finger or ear with it. | Moving one of two forms that sit close together. |
| **Move Elastic** | Like Move, but evens out the polygons around the moved area so they do not stretch. On parts above 25,000 faces it acts exactly like Move. | Big pulls on a light mesh, where Move would stretch the faces. |
| **Pull Out** | Grabs again at every dab where the last one left the surface, so one drag pulls out a long shape. The shape of the brush's tip (see
[Softness and the tip](brush-settings.md#softness-and-the-tip)) sets how it tapers. With Live Detail on, the pulled shape gets new triangles as it grows. | Horns, tentacles, fingers and other long shapes from a body. |
| **Pull Out Along Normal** | Pull Out that pulls straight out of the surface where you pressed, rather than across the screen. | Pulling a shape straight out of a surface you see at an angle. |

## Smooth

Brushes that soften and polish. The brush this family shows is the one Shift smooths with:
tap the family's button to choose it.

| Brush | What it does | Reach for it when |
|---|---|---|
| **Smooth** | Pulls each point toward its neighbours, gently: it takes a few strokes. Open edges do not shrink in. | Everyday softening between other brushes. |
| **Smooth Strong** | A stronger smooth over a wider ring of neighbours, so bumps go quickly. | Removing lumps or noise fast. |
| **Flat Polish** | Cuts what sticks up above a plane that keeps the tilt of your first press, with a crisp edge. Alt fills up to the plane instead. | Flat, hard surfaces and planes on hard forms. |
| **Smooth Polish** | A softer polish: the plane follows the form and the edge is soft. Alt fills. | Polishing curved surfaces without losing their shape. |

## Mask

Brushes that mark the surface rather than shape it. See [Masking](masking.md).

| Brush | What it does | Reach for it when |
|---|---|---|
| **Mask Pen** | Paints the mask. With Alt it paints the mask away. Holding Ctrl uses it with any brush picked. | Holding an area still while you sculpt or deform the rest. |
| **Select Rect** | Drag a rectangle: through the whole model, the faces under it stay shown and the rest are hidden. With Alt it hides the faces under it instead. Holding Shift+Ctrl uses it with any brush picked. | Hiding what is in the way, so you can work on one area. |
| **UV Density** | Paints values meant for a future automatic UV tool: where its texture detail should be denser or sparser. | Nothing yet; see below. |
| **UV Hint** | Paints values meant for a future automatic UV tool: where its seams should or should not go. | Nothing yet; see below. |

UV Density and UV Hint paint values that nothing in GigaSculpt draws or uses yet. They are there
for an automatic UV tool that is not built.

## Paint

Brushes that color the surface and leave its shape alone. See
[Looks and paint](looks.md#painting).

| Brush | What it does | Reach for it when |
|---|---|---|
| **Paint** | Paints the Main color. With Alt it paints the Secondary color. | Painting color onto the sculpt. |
| **Paint Blend** | Blends painted colors into their neighbours' and leaves the shape alone. | Softening the edge between two colors. |

Brushes outside the Paint family can paint too, while they sculpt; see
[Looks and paint](looks.md#painting).

## Brushes in other places

- **The brush wheel** (Wheel on the shelf) opens at the pointer: move toward a brush and let go
  to pick it. A right click on one of its slots puts the brush you are using there.
- **The Library** (Library on the shelf) shows every brush as a tile. Pin a brush there and it
  sits on the shelf as a favorite.
- **Back** on the shelf goes to the brush you used before; a second tap comes back.
- **The command palette** (Find on the shelf) finds a brush by typing a few letters of its
  name.
- Settings > Files > Brush at start picks whether each start begins with the brush you used
  last or with one you pinned there.
