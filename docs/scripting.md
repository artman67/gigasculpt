# Scripts and agents

Everything GigaSculpt does is a named command with typed arguments: the window's buttons, keys
and strokes run the same commands a script or an agent runs. A running GigaSculpt, with its
window or without one, is a session that listens on a socket, and `gs` talks to it.

## gs

`gs` comes with GigaSculpt and finds the `gigasculpt` program beside itself (on Windows they are
`gs.exe` and `gigasculpt.exe`, with `gigasculpt.com` for the terminal; on macOS both are inside
`GigaSculpt.app/Contents/MacOS/`). With
the window open (its session is named `main`):

```
gs commands                              # every command
gs commands sphere                       # the commands matching words
gs commands primitive.sphere             # one command's docs, arguments and examples
gs call primitive.sphere subdivisions=6  # run a command; it shows in the window at once
gs call brush.stroke --help              # a command's arguments
gs query scene.describe                  # a query, which never changes anything
gs look --views front,iso --out look.png # render offscreen, without moving your view
```

Arguments are given as `key=value`, `--key value`, a JSON object, or `@file.json`. A few options
work with every subcommand:

- `-s NAME` picks the session; without it, `gs` uses `$GS_SESSION`, else the only session
  running.
- `--json` prints the whole JSON answer instead of short text.
- `--role observer` connects with read-only rights, for queries and looks.
- `GS_CLIENT=NAME` names the client. Each step in the history is credited to whoever ran it,
  and an agent's undo takes back only its own steps.

A command that takes long (a remesh, a big subdivision) keeps running as a job after `gs`
stops waiting; `gs job ID` says how far it got, `gs job ID --wait-ms 60000` waits for it, and
`gs job ID --cancel` stops it with nothing changed.

## Sessions without a window

```
gs spawn work                       # start a headless session; prints its stop key
gs -s work call primitive.sphere
gs -s work look --out sphere.png
gs -s work stop --key KEY
```

`gigasculpt serve --headless` starts one by hand; `gigasculpt serve --help` lists its options
and the folders it uses. On Windows, type them in a terminal opened in the GigaSculpt folder
(in PowerShell, `.\gigasculpt serve --headless`), or anywhere once that folder is on your
`PATH`. There `gigasculpt` runs `gigasculpt.com`, the same program built for the console, so it
prints in that terminal and keeps it until the program ends, as on Linux and macOS;
double-clicking `gigasculpt.exe` opens the app with no console window. On Linux, offscreen
pictures in a headless session draw on lavapipe, Mesa's software Vulkan driver, unless
`--adapter` names another, so they come out the same on any machine. On Windows and macOS they
draw on the first graphics adapter the system offers.

A session's name is letters, digits, `-` and `_`. Its socket's path has a length limit (shorter
on macOS than on Linux), so a name too long for it is refused with a message that says how long
a name may be.

## Scripts and replays

A script is a JSONL file, one command per line:

```
{"command": "primitive.sphere", "args": {"subdivisions": 16}}
{"command": "brush.stroke", "args": {"brush": "Draw", "invert": true, "size": "20%", "strength": 1, "samples": [{"position": [-0.4, 0, 0.92]}, {"position": [0, 0, 1]}, {"position": [0.4, 0, 0.92]}]}}
{"command": "mask.make", "args": {"maker": {"by": "cavity"}, "blur": 1}}
```

This one carves a groove across a sphere and masks it. A stroke's points are in the scene's
units here; given a `camera`, they are screen positions instead. `gs run script.jsonl` runs it
line by line into a session and stops at the first error. Every session writes such a log of
what it ran, in the `logs` folder of GigaSculpt's state folder (see
[Files](files.md#where-gigasculpt-keeps-its-files)), so a log replayed into a fresh session
rebuilds the scene. A session started with `--verify` also
writes the scene's hash after each step, and `gs run` then names the first line that replays
differently. `gs run --files DIR` replays a log that opened or imported files, reading them
from `DIR` by their content.

## Agents over MCP

`gs mcp` serves the session to an AI agent over the Model Context Protocol, on standard input
and output. Add it to your MCP client's configuration as a command; many clients take this
form:

```json
{
  "mcpServers": {
    "gigasculpt": { "command": "gs", "args": ["mcp", "--spawn"] }
  }
}
```

With `--spawn`, `gs mcp` starts the session when it is not running (named by `-s`, else
`$GS_SESSION`, else `main`); without it, the agent works in the session you have open, and you
watch its steps land in the window. It offers six tools:

| Tool | Does |
|---|---|
| `gs_help` | Finds commands and reads their docs, arguments and examples |
| `gs_run` | Runs one command as one undo step |
| `gs_batch` | Runs several commands in order as one undo step; if one fails, none changes anything |
| `gs_query` | Runs a query, which only reads |
| `gs_look` | Renders the scene offscreen as a PNG, one view or a labelled sheet of several |
| `gs_job` | Follows or cancels a long command |
