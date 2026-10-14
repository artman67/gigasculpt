# The icon

`icon.svg` is the source: a rounded square in the interface's grays, on macOS's icon grid (an
824-pixel body in a 1024-pixel canvas, with a soft shadow), with the logo, `logo.png`, laid on
it. Every other file here is made from it by `make.py`:

- `icon-16.png` to `icon-1024.png`: the window's icon (`icon-256.png`, built into gs-platform),
  the Linux desktop icon and the README's;
- `icon.ico`: the Windows icon, PNG entries of 16 to 256 pixels;
- `icon.icns`: the macOS icon, PNG entries of 16 to 1024 pixels.

## Changing the icon

1. Replace `logo.png` (a square picture with a transparent background, as the next section
   makes one), or edit `icon.svg`. The logo fills the body (`x`, `y`, `width` and `height` of
   its `<image>`), clipped to its rounded corners.
2. Run `python3 assets/brand/make.py` (needs `rsvg-convert`, from librsvg).
3. Commit the source and every file it wrote: the release builds read them and have no
   `rsvg-convert`.

A detailed logo reads as a blob at 16 and 32 pixels. A simpler `icon-small.svg` beside
`icon.svg`, when it exists, gives those sizes instead (every size up to 32 pixels).

## The logo from a scene

GigaSculpt draws the logo itself, from a scene file, in a session without a window. From the
repository's top folder, with `gigasculpt` and `gs` built and `gs` on your `PATH` (or written
as `target/release/gs`; it finds `gigasculpt` beside it):

```
gs spawn icon
gs -s icon call file.open path=$PWD/head.gss
gs -s icon call view.snapshot view=front size=1024 transparent=true path=$PWD/assets/brand/logo.png
gs -s icon stop --key KEY
python3 assets/brand/make.py
```

1. `gs spawn icon` starts a session named `icon` and prints its stop key, the `KEY` of the
   fourth line.
2. `file.open` opens the scene. Give paths in full: the session does not run in this folder.
3. `view.snapshot` draws it 1024 pixels square from the front, framing every part, in the
   default look (MatCap Gray), with no background: where no part is, the picture is
   transparent, and along the edges partly so.
4. `stop` ends the session, and `make.py` writes every icon file from the new logo.

For another angle, `view` also takes `back`, `left`, `right`, `top`, `bottom` or `iso`, or the exact
camera of the window's view: with the window open, `gs query view.get view=main --json` prints
it as `camera`; save that object as `camera.json` and give `view=@camera.json`. A look set
before the snapshot, such as `material.select name=red-wax`, draws as it does in the window.
