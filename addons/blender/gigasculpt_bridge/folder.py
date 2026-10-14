# SPDX-License-Identifier: GPL-3.0-or-later
"""Exchange folders (decision 0032, section 1): where they are, and their files.

A folder holds one raw little-endian file per column and `manifest.json`, written last, so a
reader that finds the manifest finds every file it lists. GigaSculpt writes `to-blender/` and
reads `from-blender/`, both under `$XDG_CACHE_HOME/gigasculpt/exchange/<session>/`.
"""

import json
import os
import re

import numpy as np

from . import names

MANIFEST = "manifest.json"
FORMAT = "gigasculpt-exchange"
VERSION = 1
WRITER = "GigaSculpt bridge for Blender 0.1.0"

_SESSION = re.compile(r"[A-Za-z0-9_-]+")
_PLAIN = re.compile(r"[A-Za-z0-9._-]+")


class ExchangeError(Exception):
    """Something the user can act on, said in a sentence."""


def check_session(session):
    """A session name is one or more ASCII letters, digits, `-` and `_`, as the engine says."""
    if not _SESSION.fullmatch(session or ""):
        raise ExchangeError(f"bad session name '{session}': use letters, digits, '-' and '_'")


def root(session):
    """The session's exchange folder. An empty or relative XDG_CACHE_HOME counts as unset."""
    check_session(session)
    base = os.environ.get("XDG_CACHE_HOME", "")
    if not os.path.isabs(base):
        base = os.path.join(os.path.expanduser("~"), ".cache")
    return os.path.join(base, "gigasculpt", "exchange", session)


def side(session, name):
    """`to-blender` or `from-blender` of the session, made if missing."""
    path = os.path.join(root(session), name)
    os.makedirs(path, exist_ok=True)
    return path


def clear(path):
    """Removes the files directly in `path` (never folders), so no old file is read again."""
    for name in os.listdir(path):
        full = os.path.join(path, name)
        if os.path.isfile(full) or os.path.islink(full):
            os.remove(full)


def plain(name):
    """A plain file name in the folder, so a manifest can never name a file elsewhere."""
    return bool(_PLAIN.fullmatch(name)) and not name.startswith(".")


def read_manifest(path):
    """The folder's manifest, checked for the format, the version and the name table."""
    with open(os.path.join(path, MANIFEST), "rb") as f:
        manifest = json.loads(f.read())
    if manifest.get("format") != FORMAT or manifest.get("version") != VERSION:
        raise ExchangeError(
            f"{path} holds {manifest.get('format')} version {manifest.get('version')}; this "
            f"add-on reads {FORMAT} version {VERSION}. Update the add-on or GigaSculpt."
        )
    for entry in manifest["parts"]:
        for column in entry["columns"]:
            if names.row(column) is None:
                raise ExchangeError(
                    f"part '{entry['name']}' lists the column {column.get('blender')} as "
                    f"{column.get('gs')}, which this add-on's name table does not have. "
                    "Update the add-on to match GigaSculpt."
                )
            if not plain(column["file"]):
                raise ExchangeError(f"the manifest names the file {column['file']!r}")
    return manifest


def columns(path, entry):
    """Every column file of a manifest entry, by Blender name, as numpy arrays: (count, width)
    for a width above one, else (count,)."""
    counts = {"point": entry["points"], "face": entry["faces"], "corner": entry["corners"]}
    out = {}
    for column in entry["columns"]:
        c = names.row(column)
        count = counts[c.domain]
        values = np.fromfile(os.path.join(path, column["file"]), dtype=names.DTYPE[c.type])
        if values.size != count * c.width:
            raise ExchangeError(
                f"part '{entry['name']}': {column['file']} has {values.size} values, "
                f"{count * c.width} expected"
            )
        out[c.blender] = values.reshape(count, c.width) if c.width > 1 else values
    return out


def put_column(path, k, c, values):
    """Writes column `c` of the `k`th object and returns its manifest column."""
    file = f"{k}-{c.blender.lstrip('.')}.bin"
    np.ascontiguousarray(values, dtype=names.DTYPE[c.type]).tofile(os.path.join(path, file))
    return names.entry(c, file)


def put_manifest(path, parts):
    """Writes the manifest last, through a file beside it, so a reader never sees half of it."""
    manifest = {"format": FORMAT, "version": VERSION, "writer": WRITER, "parts": parts}
    temp = os.path.join(path, f".{MANIFEST}.{os.getpid()}.part")
    with open(temp, "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=1)
    os.replace(temp, os.path.join(path, MANIFEST))


def remove_listed(path, manifest):
    """Removes the manifest and the files it lists, once they are loaded."""
    os.remove(os.path.join(path, MANIFEST))
    for entry in manifest["parts"]:
        for column in entry["columns"]:
            full = os.path.join(path, column["file"])
            if os.path.exists(full):
                os.remove(full)
