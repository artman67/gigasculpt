# Troubleshooting

If something here does not help, [report a bug](#reporting-a-bug). The Windows and macOS builds
are new and less tested than Linux, so reports from those systems are especially welcome.

## Pen pressure does nothing

- **Check the brush.** The pen toggles at the right of Size and Strength in the brush tray say
  whether pressure changes them for this brush. The Move brushes and Pull Out keep one size and
  strength for the whole drag on purpose, so pressure never changes them.
- **Check the settings.** In Settings > Pen, Sensitivity at 0 leaves pressure out for every
  brush. See [Settings](settings.md#pen) and [Pen pressure](brush-settings.md#pen-pressure).
- **Check the tablet's driver.** A tablet set to act as a mouse sends no pressure. Set it to pen
  mode in its own settings.
- **On Linux, try the other video driver.** The pen reaches GigaSculpt through your desktop's
  display system. Start the app with `gigasculpt window --driver x11` or
  `gigasculpt window --driver wayland` to try the other one, and say which you used if you
  report a bug.

## The window does not open, or draws wrongly

GigaSculpt draws with your graphics card. On Linux it needs a working Vulkan driver: update your
graphics driver first.

To see why the window will not open, start it from a terminal and read what it prints:

- on Linux, run `gigasculpt`;
- on macOS, run `/Applications/GigaSculpt.app/Contents/MacOS/gigasculpt`;
- on Windows, the app prints nothing to a terminal yet, so a failed start shows no reason.
  [Report it](#reporting-a-bug) with your graphics card's name.

With more than one graphics device, GigaSculpt picks the fastest one that can draw to the
window. On Linux you can choose another with `--adapter`:

```
gigasculpt window --adapter nvidia     # an NVIDIA card with NVIDIA's own driver
gigasculpt window --adapter radv       # an AMD card with Mesa's driver
gigasculpt window --adapter lavapipe   # no graphics card at all: slow, but it always works
```

## First start on Windows and macOS

The builds are not signed yet, so each system warns the first time you open GigaSculpt.

- **Windows:** if SmartScreen says it protected your PC, click "More info", then "Run anyway".
- **macOS:** open GigaSculpt once and let macOS refuse it. Then open System Settings > Privacy &
  Security, scroll down, click "Open Anyway" beside GigaSculpt and confirm. From then on it opens
  normally. From a terminal, `xattr -dr com.apple.quarantine /Applications/GigaSculpt.app` does
  the same.

## A session name is refused

A session's name becomes part of a file path, and each system limits how long that path may be
(macOS allows less than Linux). A name that is too long is refused with a message saying so;
choose a shorter one. Names may use letters, digits, `-` and `_`.

## Nothing prints on Windows

On Windows, `gigasculpt.exe` started from a terminal prints nothing, not even `--version` or
`--help`. Use `gs.exe`, which comes beside it, for anything you run from a terminal (see
[Scripts and agents](scripting.md)).

## The Blender add-on cannot find GigaSculpt

Start GigaSculpt first, and check that the add-on's Session matches the session's name (`main`
unless you chose another). The add-on does not work on Windows yet. See
[The Blender add-on](blender.md).

## Reporting a bug

Open an issue on the project's GitHub page and pick the bug template. A good report says:

- **The version:** `gigasculpt --version` prints it (on Windows, `gs --version`).
- **Your system:** Linux, Windows or macOS and its version, your graphics card, and your tablet.
- **What you did, what you expected and what happened.** The fewer steps it takes to make it
  happen again, the faster it can be fixed.

**The log helps most.** Every session writes a log of the commands it ran, in the `logs` folder
of GigaSculpt's state folder:

| System | Logs folder |
|---|---|
| Linux | `~/.local/state/gigasculpt/logs` |
| macOS | `~/Library/Application Support/GigaSculpt/logs` |
| Windows | `%LOCALAPPDATA%\GigaSculpt\logs` |

Each file is named after its session and process, such as `main-12345.jsonl`; the newest one is
usually the one you want. It is a plain text list of the commands the session ran, with their
settings, and replaying it rebuilds the scene step by step, which is often enough to find the
bug. Look it over before you attach it if your file names are private.

**Bug bundles.** When a command crashes inside GigaSculpt (rather than refusing with a message),
GigaSculpt writes a bug bundle, a folder ending in `.gsrepro`, beside the log, and the error
message says where. The bundle holds a starting scene, every step after it up to the crash, and
a screenshot of the window when one could be taken, so anyone can replay the bug exactly. Zip it and attach it if you
are happy to share the scene. `gs repro minimize` shrinks a bundle to the steps the bug needs.

For contributors, [CONTRIBUTING](../CONTRIBUTING.md) has more on bug reports.
