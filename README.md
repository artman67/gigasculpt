# GigaSculpt

GigaSculpt is a free, open-source digital sculpting app for pen and mouse. It is written in Rust,
draws with wgpu, and is licensed under the GNU GPL, version 3 or later.

![The Workbench with a sculpted head, the brush shelf on top, the brush tray on the left and the parts on the right](docs/images/workbench.png)

## Status

GigaSculpt is early. It opens, sculpts, saves and loads, and everything listed below works
today, but expect rough edges, missing features and changes to the scene file format between
versions.

Linux comes first: it is where GigaSculpt is built and tested on every change. Windows and macOS
builds are made from the same code for every release; they are new and less tested.

## What works today

- **Sculpting brushes.** 24 brushes in six families on the shelf: Build (Draw, Clay, Clay
  Build, Clay Line, Inflate), Carve (Trim, Clip Curve, Pinch, Sharp Crease), Move (Move, Move
  Elastic, Move Connected, Pull Out and Pull Out Along Normal), Smooth (four of them), Mask
  (Mask Pen, Select Rect and two UV helpers) and Paint (Paint, Paint Blend). Pen pressure,
  alphas, stroke smoothing and every brush setting sit in the brush tray.
- **Live Detail.** The mesh adds and removes triangles under the brush as you sculpt, so a form
  can grow out of a sphere without planning its mesh first. Backslash turns it on and off.
- **Subdivision levels.** Subdivide a part and step between its levels, to work on big forms
  low and on fine detail high.
- **Remeshing.** Voxel Remesh rebuilds a part as an even mesh, closing holes and joining
  overlaps. Quad Remesh rebuilds it as quads in flowing rows to a face count (Linux only for
  now, with a helper program you build yourself; see [Building](docs/building.md)). Decimate
  and Project Detail are in the Mesh menu too.
- **Parts.** A scene holds many parts. Add spheres, cubes and cylinders; duplicate, mirror,
  rename, split, extract and merge parts; show and hide them from the Parts list.
- **Masking.** Paint a mask with Ctrl held, then invert, grow, shrink, blur or sharpen it, mask
  by cavity or by features, hide what is masked, and turn a mask into a face group.
- **Symmetry.** Mirror, parallel and radial symmetry on each axis, and Mirror and weld to copy
  one side of a part onto the other.
- **Deform.** Offset, rotate, stretch, flatten, spherize, bend, twist, taper, noise and more,
  on the whole part or what is left unmasked.
- **Materials and looks.** 20 MatCaps, 5 lighting environments, color painting, and a Library
  of brushes, alphas, materials, environments, layouts and themes (dark, light and high
  contrast).
- **The Workbench interface.** The brush shelf along the top, the brush tray on the left, the
  scene tray on the right, a quick menu under the pointer, a command palette that finds every
  command, and an adjust panel that changes the last command's settings after the fact.
  Panels can be rearranged and saved, and any button can get a key of its own.
- **Files.** Scenes and single parts save as `.gss` files, with autosave and crash recovery.
  OBJ, PLY and STL files import and export.
- **Blender exchange.** An add-on for Blender 5.2 or later moves parts between a running
  GigaSculpt and Blender, both ways.
- **Scripts and agents.** Every action in the app is a named command. The `gs` command-line
  tool runs commands against a running session, replays scripts and renders pictures
  offscreen, and `gs mcp` serves the same commands to AI agents over the Model Context
  Protocol. See [Scripts and agents](docs/scripting.md).

## A look around

The Build family open on the shelf, and the brush tray with the brush's settings:

![The brush shelf with the Build family open, and the brush tray on the left](docs/images/brush-tray.png)

The quick menu, opened at the pointer with Shift+Space:

![The quick menu beside the model with the brush's size, strength and softness, symmetry, the mask and the history](docs/images/quick-menu.png)

The same head in the Red Wax MatCap, with the panels hidden (Tab):

![The head in the Red Wax MatCap with the panels hidden](docs/images/look-red-wax.png)

## Install

Download the file for your system from the [Releases](../../releases) page:

| System | File | Then |
|---|---|---|
| Linux (x86_64) | `gigasculpt-VERSION-linux-x86_64.tar.gz` | Unpack it and run `./gigasculpt` in the folder. |
| Windows (x86_64) | `gigasculpt-VERSION-windows-x86_64.zip` | Unpack it and run `gigasculpt.exe`. |
| macOS (Apple silicon and Intel) | `gigasculpt-VERSION-macos.dmg` | Open it and drag GigaSculpt to Applications. |
| Blender add-on | `gigasculpt-blender-addon-VERSION.zip` | In Blender 5.2 or later: Edit > Preferences > Add-ons, then Install from Disk in the menu at the top right. |

Keep `gigasculpt` and `gs` in the same folder: `gs` finds the app beside itself. On Linux, put
that folder on your `PATH` to use `gs` from anywhere. GigaSculpt needs a graphics driver with
Vulkan on Linux (Mesa or your card maker's), DirectX 12 or Vulkan on Windows, and Metal on macOS.
The Linux build is made on Ubuntu 24.04 and needs glibc 2.39 or newer.

The builds are not signed yet, so the first start shows a warning:

- **Windows:** SmartScreen says it protected your PC. Click "More info", then "Run anyway".
- **macOS:** the first open is refused. Open System Settings > Privacy & Security, scroll down
  and click "Open Anyway" beside GigaSculpt, then open it again.

## Build from source

You need Rust through [rustup](https://rustup.rs) (the repository's `rust-toolchain.toml` picks
the version), CMake and a C compiler, as SDL3 is built from source. On Linux you also need the
X11 and Wayland development headers. Then:

```
cargo build --release --locked
target/release/gigasculpt
```

[Building](docs/building.md) lists the packages for each system, the tests and Quad Remesh's
helper program.

## First steps

1. Run `gigasculpt`. The window opens on the Workbench with an empty scene.
2. Choose Add > Sphere. The adjust panel shows its settings; change them or carry on.
3. Press Backslash to turn on Live Detail, or Ctrl+D to subdivide, so the sphere has room for
   detail.
4. Pick a brush on the shelf at the top and drag across the sphere with the pen or the left
   mouse button. Hold Shift to smooth, Ctrl to paint a mask, and Alt to work the other way
   (dig instead of build).
5. Drag on the empty canvas or with the right mouse button to turn the view. Middle drag pans,
   the wheel zooms toward the point under the pointer, and F frames the model.
6. Press Ctrl+S to save the scene.

A few keys:

| Key | Does |
|---|---|
| Space | Command palette: type to find any command |
| Shift+Space | Quick menu: brush size, strength and softness, symmetry, mask |
| Ctrl+Z, Ctrl+Shift+Z | Undo, redo |
| S, U, O (held, then drag) | Brush size, strength, softness |
| [ and ] | Brush size down and up |
| X | Mirror across X on and off |
| F | Frame every part; press again for the active one |
| Ctrl+I, Alt+M | Invert the mask, clear it |
| Ctrl+D, D, Shift+D | Subdivide, one level up, one level down |
| Ctrl+R | Voxel Remesh |
| Comma | Library |
| Tab | Hide and show the panels |

[Getting started](docs/getting-started.md) and [The interface](docs/interface.md) go further.

## Documentation

- [Getting started](docs/getting-started.md): a first sculpt, saving, and where files go.
- [The interface](docs/interface.md): the Workbench's parts, navigation and the keys.
- [Building](docs/building.md): building from source, the tests and the lints.
- [Scripts and agents](docs/scripting.md): `gs`, scripts, replays and `gs mcp`.

## License

GigaSculpt is free software under the GNU General Public License, version 3 or (at your option)
any later version. See [LICENSE](LICENSE). Quad Remesh's helper program is built from other
open-source projects under their own licenses, which its build keeps beside it.

## Contributing

Bug reports, ideas and pull requests are welcome. [CONTRIBUTING.md](CONTRIBUTING.md) says how to
build, test and send a change, and how to report a bug.
