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
- on Windows, open a terminal in the GigaSculpt folder and type `gigasculpt` (in PowerShell,
  `.\gigasculpt`), which runs `gigasculpt.com` and prints there.

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

`gigasculpt.exe` is the app you double-click, and it prints nothing, not even `--version` or
`--help`. In a terminal, type `gigasculpt` without `.exe` (`.\gigasculpt` in PowerShell), in the
GigaSculpt folder or anywhere once that folder is on your `PATH`: that runs `gigasculpt.com`, the
same program built for the terminal, which prints there (see
[Scripts and agents](scripting.md#sessions-without-a-window)).

## A script on Windows waits after `gs spawn`

A known issue on Windows: the session `gs spawn` starts keeps a copy of `gs`'s output open. So
a program that reads `gs spawn`'s output through a pipe until it ends, such as `$(gs spawn work)`
in Git Bash, `$s = gs spawn work` in PowerShell or Python's `subprocess` with
`capture_output`, waits until the session stops. Typing `gs spawn` in a terminal is not
affected. In a script, send its output to a file and read the stop key from there:

```
gs spawn work > spawn.txt
gs -s work stop --key "$(sed -n 's/^stop key: \([^ ]*\).*/\1/p' spawn.txt)"
```

In Python, pass an open file as `stdout` instead of capturing it. A program that captures the
output of the whole script still waits for the session, so have the script stop the session
before it ends.

`gs mcp --spawn` does the same with the session it starts: that session keeps the MCP client's
connection to `gs mcp` open, so a client that waits for `gs mcp` to end waits for the session
too. Open GigaSculpt first, or start the session from a terminal, and leave out `--spawn`: then
`gs mcp` uses the session that is running and starts nothing.

## A download does not start

Keep everything a download holds together in one folder, as it came: `gs` looks for the app
beside itself and the app for the `quadwild` folder.

- **Linux:** a message that a version of `GLIBC` was not found means the system is older than
  the download is built for: it needs glibc 2.35 or newer (Ubuntu 22.04, Debian 12 and later).
  On an older system, [build it from source](building.md).
- **Windows:** a message that `VCRUNTIME140.dll` or `MSVCP140.dll` was not found means a program
  was moved out of its folder, away from the copies the download holds beside it. Move it back,
  or install Microsoft's Visual C++ Redistributable (x64).

## The Blender add-on cannot find GigaSculpt

Start GigaSculpt first, and check that the add-on's Session matches the session's name (`main`
unless you chose another). On Windows it needs Windows 10 version 1803 or later. See
[The Blender add-on](blender.md).

## Reporting a bug

Open an issue on the project's GitHub page and pick the bug template. A good report says:

- **The version:** `gigasculpt --version` prints it (on Windows in a terminal, as above), and
  `gs --version` prints the same.
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
