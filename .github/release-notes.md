GigaSculpt is early: expect rough edges, and scene files may change between versions. Linux is
built and tested on every change; the Windows and macOS builds are new and less tested.

## Downloads

| File | For |
|---|---|
| `gigasculpt-VERSION-linux-x86_64.tar.gz` | Linux on x86_64, glibc 2.39 or newer (Ubuntu 24.04, Debian 13, Fedora 40 and later), with a Vulkan driver |
| `gigasculpt-VERSION-windows-x86_64.zip` | Windows 10 and 11 on x86_64 |
| `gigasculpt-VERSION-macos.dmg` | macOS 11 or later, Apple silicon and Intel |
| `gigasculpt-blender-addon-VERSION.zip` | Blender 5.2 or later: Edit > Preferences > Add-ons, then Install from Disk |

Each of the three app downloads holds two programs: `gigasculpt`, the app, and `gs`, its
command-line client and MCP server. Keep them in the same folder; on macOS `gs` is inside the app, at
`GigaSculpt.app/Contents/MacOS/gs`. Quad Remesh's helper program is not included; see
`docs/building.md` to build it (Linux only for now).

## Opening an unsigned app

These builds are not signed yet, so each system warns the first time.

- **Linux:** unpack the archive and run `./gigasculpt`. Nothing more is needed.
- **Windows:** unpack the zip and run `gigasculpt.exe`. If SmartScreen says it protected your
  PC, click "More info", then "Run anyway".
- **macOS:** open the disk image and drag GigaSculpt to Applications. Open it once; macOS
  refuses. Then open System Settings > Privacy & Security, scroll down, click "Open Anyway"
  beside GigaSculpt and confirm. From then on it opens normally. (From a terminal,
  `xattr -dr com.apple.quarantine /Applications/GigaSculpt.app` does the same.)
