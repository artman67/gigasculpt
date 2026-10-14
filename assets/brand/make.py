#!/usr/bin/env python3
"""Writes every icon file from icon.svg, beside this script, with rsvg-convert.

    python3 assets/brand/make.py

Writes icon-<size>.png (16 to 1024 pixels), icon.ico (Windows: PNG entries of 16 to 256 pixels)
and icon.icns (macOS: PNG entries of 16 to 1024 pixels). When icon-small.svg exists beside
icon.svg, the sizes up to SMALL pixels come from it instead: a simpler picture that still reads
at 16 and 32 pixels. Each PNG keeps only the chunks a picture needs, so no file carries a path,
a name or a date. Standard library only.
"""

import shutil
import struct
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
PNG_SIZES = [16, 32, 48, 64, 128, 256, 512, 1024]
ICO_SIZES = [16, 24, 32, 48, 64, 256]
# macOS's icon types that hold a PNG, and their sizes (ic10 is 512 at twice the density).
ICNS_TYPES = [(b"icp4", 16), (b"icp5", 32), (b"icp6", 64), (b"ic07", 128), (b"ic08", 256),
              (b"ic09", 512), (b"ic10", 1024)]
SMALL = 32
SIGNATURE = b"\x89PNG\r\n\x1a\n"
KEPT = {b"IHDR", b"PLTE", b"tRNS", b"IDAT", b"IEND"}


def source(size):
    small = HERE / "icon-small.svg"
    return small if size <= SMALL and small.is_file() else HERE / "icon.svg"


def bare(png):
    """`png` with its pixels and nothing else: every chunk but KEPT left out."""
    if not png.startswith(SIGNATURE):
        raise SystemExit("make.py: rsvg-convert wrote no PNG")
    chunks, at = [SIGNATURE], len(SIGNATURE)
    while at < len(png):
        length = int.from_bytes(png[at:at + 4], "big")
        if png[at + 4:at + 8] in KEPT:
            chunks.append(png[at:at + 12 + length])
        at += 12 + length
    return b"".join(chunks)


def render(size):
    """The icon at `size` pixels square, as PNG bytes."""
    command = ["rsvg-convert", "-w", str(size), "-h", str(size), str(source(size))]
    return bare(subprocess.run(command, check=True, capture_output=True).stdout)


def ico(pngs):
    """A Windows icon holding `pngs`, (size, PNG bytes) pairs; a size of 256 is written 0."""
    header = struct.pack("<HHH", 0, 1, len(pngs))
    offset = len(header) + 16 * len(pngs)
    entries = []
    for size, png in pngs:
        entries.append(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(png),
                                   offset))
        offset += len(png)
    return header + b"".join(entries) + b"".join(png for _, png in pngs)


def icns(pngs):
    """A macOS icon holding `pngs`, (type, PNG bytes) pairs."""
    body = b"".join(kind + struct.pack(">I", 8 + len(png)) + png for kind, png in pngs)
    return b"icns" + struct.pack(">I", 8 + len(body)) + body


def main():
    if shutil.which("rsvg-convert") is None:
        raise SystemExit("make.py: needs rsvg-convert (librsvg)")
    sizes = sorted({*PNG_SIZES, *ICO_SIZES, *(size for _, size in ICNS_TYPES)})
    pngs = {size: render(size) for size in sizes}
    for size in PNG_SIZES:
        (HERE / f"icon-{size}.png").write_bytes(pngs[size])
    (HERE / "icon.ico").write_bytes(ico([(size, pngs[size]) for size in ICO_SIZES]))
    (HERE / "icon.icns").write_bytes(icns([(kind, pngs[size]) for kind, size in ICNS_TYPES]))
    print(f"make.py: {len(PNG_SIZES)} PNGs, icon.ico and icon.icns in {HERE.name}/")


if __name__ == "__main__":
    main()
