*mog the other sculpting programs*

GigaSculpt is early: expect rough edges, and scene files may change between versions. Linux is
built and tested on every change; the Windows and macOS builds are new and less tested.

**0.1.1** fixes the app on Windows and macOS: in 0.1.0 its window could not draw there, so the
app closed as soon as it was opened. If the app cannot start now, it says why in a message box.
On Windows, a script reading `gs spawn` or `gs mcp --spawn` no longer waits for the session to stop.
When a file cannot be read or written, the app and `gs` say why in plain words (no file there, a
folder, a file GigaSculpt may not read or write) instead of the system's message and error number.

## Downloads

| File | For |
|---|---|
| `gigasculpt-VERSION-linux-x86_64.tar.gz` | Linux on x86_64, glibc 2.35 or newer (Ubuntu 22.04, Debian 12, Fedora 36 and later), with a Vulkan driver |
| `gigasculpt-VERSION-windows-x86_64.zip` | Windows 10 and 11 on x86_64 |
| `gigasculpt-VERSION-macos.dmg` | macOS 11 or later, Apple silicon and Intel |
| `gigasculpt-blender-addon-VERSION.zip` | Blender 5.2 or later: Edit > Preferences > Add-ons, then Install from Disk |
| `quadwild-bimdf-PIN-source.tar.gz` | The source of Quad Remesh's helper program and the scripts that build it, as its GPL license asks; nothing to install |

Each of the three app downloads holds two programs, `gigasculpt`, the app, and `gs`, its
command-line client and MCP server, and Quad Remesh's helper program, built for that system, in
a `quadwild` folder. Keep them together in one folder. On macOS they are inside the app: `gs` at
`GigaSculpt.app/Contents/MacOS/gs`, the helper in `GigaSculpt.app/Contents/Resources/quadwild`.
On Windows there is also `gigasculpt.com`, the same app built for the terminal: typing
`gigasculpt` in a terminal in that folder (`.\gigasculpt` in PowerShell) runs it, so
`gigasculpt --help` and `gigasculpt serve --headless` print there and the terminal waits. Beside
them are the Microsoft C and C++ runtime files they use, so nothing else needs installing. Each
download also holds a short `README.txt` that says what it is, how to start it and where the
guide is.

Before a release is published, each download is unpacked on its own system as a person gets it
(the Linux one on Ubuntu 22.04 and Debian 12 with nothing added) and started there: both
programs print this version and their help, the app's window opens and draws (on Windows and
macOS), and Quad Remesh's helper remeshes a small sphere.

## Opening an unsigned app

These builds are not signed yet, so each system warns the first time.

- **Linux:** unpack the archive and run `./gigasculpt`. Nothing more is needed. To find it in
  your app menu, put `gigasculpt` and `gs` in `~/.local/bin`, `gigasculpt.desktop` in
  `~/.local/share/applications` and `gigasculpt.png` in `~/.local/share/icons/hicolor/256x256/apps`.
- **Windows:** unpack the zip and run `gigasculpt.exe` (its type is "Application";
  `gigasculpt.com` is the terminal's). If SmartScreen says it protected your PC, click "More
  info", then "Run anyway".
- **macOS:** open the disk image and drag GigaSculpt to Applications. Open it once; macOS
  refuses. Then open System Settings > Privacy & Security, scroll down, click "Open Anyway"
  beside GigaSculpt and confirm. From then on it opens normally. (From a terminal,
  `xattr -dr com.apple.quarantine /Applications/GigaSculpt.app` does the same.)
