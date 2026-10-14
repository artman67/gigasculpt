# SPDX-License-Identifier: GPL-3.0-or-later
"""What the test scripts share. The engine's `tier2` tests run each script in a headless Blender
of its own and read the JSON on its `RESULT` line. The add-on's folder goes on `sys.path`, so
the scripts import the add-on's own modules: the tests check what users run."""

import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))

import bpy  # noqa: E402


def args(count):
    """The script's arguments, after `--`."""
    out = sys.argv[sys.argv.index("--") + 1 :]
    if len(out) != count:
        raise SystemExit(f"{len(out)} arguments, {count} expected")
    return out


def empty_scene():
    """Removes the factory startup's objects, so only what the exchange makes is there."""
    for ob in list(bpy.data.objects):
        bpy.data.objects.remove(ob)


def result(value):
    """Prints the line the engine test reads."""
    print("RESULT " + json.dumps(value), flush=True)
