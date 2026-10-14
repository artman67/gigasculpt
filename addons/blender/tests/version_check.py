# SPDX-License-Identifier: GPL-3.0-or-later
"""One version (decision 0064): the workspace's (`[workspace.package] version` in Cargo.toml,
which `gigasculpt --version` and `gs --version` print), the add-on's manifest and its `bl_info`
are one number, and so is a release's tag when one is given. Plain Python 3.11 or later, no
Blender: CI runs it on every push, and the release workflow before it builds and on the add-on
as its zip unpacks.

    python3 -I addons/blender/tests/version_check.py [--tag vX.Y.Z] [--addon DIR]

Prints the version and exits 0 when they agree; else says on stderr which differ and exits 1.
"""

import argparse
import ast
import pathlib
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[3]
ADDON = ROOT / "addons" / "blender" / "gigasculpt_bridge"


def toml(path):
    with open(path, "rb") as f:
        return tomllib.load(f)


def bl_info_version(init):
    """`bl_info`'s version in the add-on's `__init__.py`, read without importing it (it needs
    Blender), as "X.Y.Z"; None without one."""
    for node in ast.parse(init.read_text(encoding="utf-8")).body:
        if isinstance(node, ast.Assign) and [getattr(t, "id", "") for t in node.targets] == ["bl_info"]:
            return ".".join(str(n) for n in ast.literal_eval(node.value).get("version", ()))
    return None


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--tag", help="a release tag, v and the version")
    parser.add_argument("--addon", type=pathlib.Path, default=ADDON,
                        help="the add-on's folder (default: the repository's)")
    args = parser.parse_args(argv)
    version = toml(ROOT / "Cargo.toml")["workspace"]["package"]["version"]
    found = {
        "the add-on's blender_manifest.toml": toml(args.addon / "blender_manifest.toml").get("version"),
        "the add-on's bl_info": bl_info_version(args.addon / "__init__.py"),
    }
    if args.tag is not None:
        found[f"the tag {args.tag}"] = args.tag.removeprefix("v")
    wrong = [f"{name} says {said}" for name, said in found.items() if said != version]
    if wrong:
        print(f"version_check: the workspace version is {version}, but " + "; ".join(wrong),
              file=sys.stderr)
        return 1
    print(version)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
