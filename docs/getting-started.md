# Getting started

This page takes you from a sphere to a first sculpt, then saves it. [The interface](interface.md)
explains each part of the window, and [Keys](keys.md) lists every key.

## Open the app

Run `gigasculpt` (on Windows `gigasculpt.exe`, on macOS GigaSculpt in Applications). With no
arguments it opens the window on the Workbench with an empty scene. A few options, given after
`window`:

```
gigasculpt window --width 1600 --height 1000   # the window's size in points
gigasculpt window --driver x11                   # Linux: SDL's video driver, x11 or wayland
gigasculpt window --no-autosave                  # no autosave and no crash recovery
gigasculpt window --session studio               # a session name other than "main"
```

`gigasculpt window --help` lists them all, and `gigasculpt --version` prints the version. On
Windows the app prints nothing in a terminal, so use `gs --version` there instead (`gs` comes
with the app; see [Scripts and agents](scripting.md)).

The first time you open GigaSculpt on Windows or macOS, the system may warn you about it;
[Troubleshooting](troubleshooting.md#first-start-on-windows-and-macos) says what to do.

## A first sculpt

1. **Add a sphere.** Choose Add > Sphere. The adjust panel opens with the sphere's settings
   (its form, density and detail levels); change them or carry on.
2. **Give it room for detail.** In the Detail section of the scene tray on the right, click On
   beside Live to turn on Live Detail, which adds triangles under the brush as you sculpt. Or
   click Subdivide once or twice to give the whole part more faces. [Detail and
   remeshing](detail.md) explains both.
3. **Build up a form.** On the shelf at the top, open the Build family and pick Clay Build.
   Drag across the sphere with the pen or the left mouse button. Hold Alt while you drag to dig
   in instead of building up.
4. **Move big shapes.** Pick Move from the Move family and drag to pull the form around. Change
   the brush's size and strength with the sliders in the brush tray on the left. Rest the
   pointer on a brush to see its key.
5. **Smooth.** Hold Shift while you drag to smooth, whatever brush is picked.
6. **Turn on symmetry.** Click X beside Symmetry at the right end of the header, so every stroke
   is mirrored across X. See [Symmetry](symmetry.md).
7. **Mask.** Hold Ctrl and drag to paint a mask; masked areas are held still by brushes and
   deformers. With Ctrl held, a tap on the empty canvas inverts the mask and a drag there
   clears it. The Mask menu has the rest; see [Masking](masking.md).
8. **Change the look.** Open Look at the right end of the header and pick a MatCap.

   ![The Look options open from the header, with the head in the Skin 4 MatCap](images/look-skin.png)

Ctrl+Z undoes and Ctrl+Shift+Z redoes. The History section of the scene tray on the right names
the step each would take.

## Parts

A scene holds any number of parts. Add another with Add > Sphere, Cube or Cylinder; the Parts
list at the top of the scene tray shows them all. Click a part's name to make it the one you
sculpt, and click its Visible button to hide or show it. [Parts, the Gizmo and Deform](parts.md)
says more.

![The Parts list with five parts, the left eye picked](images/parts.png)

## Saving

Choose File > Save (Ctrl+S). The first time, your system's file browser asks where to save;
GigaSculpt scenes are `.gss` files. Later saves write the same file and keep the old one as a
backup. The window's title names the file and shows when there are unsaved changes, and closing
the window with unsaved changes asks whether to save them first.

File > Open scene (Ctrl+O) opens a scene again. You can also drop a scene file on the window, or
give its path when you start the app: `gigasculpt head.gss`.

While you work, GigaSculpt also autosaves a copy now and then, and if it ever stops
unexpectedly, the next start offers your work back. [Files](files.md) covers saving, autosave,
recovery, importing and exporting, and where GigaSculpt keeps its files.

## Next

- [The interface](interface.md) for a tour of the window.
- [Brushes](brushes.md) for what each brush does.
- [Blender](blender.md) to move parts to and from Blender.
