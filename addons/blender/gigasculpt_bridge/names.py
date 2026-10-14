# SPDX-License-Identifier: GPL-3.0-or-later
"""The one name table (decision 0032, section 3), mirrored from gs-engine `exchange/names.rs`.

Every column the exchange carries, with GigaSculpt's name, Blender's, the domain, the element
type of its file and the values per element. The manifest repeats both names for every column,
and `row` refuses a column that does not match this table exactly, so a table changed on one
side only fails loudly instead of putting values under the wrong name.

The engine does the axis change and the crease values, so nothing here does arithmetic.
"""

from collections import namedtuple

Column = namedtuple("Column", "gs blender domain type width")

POSITION = Column("positions", "position", "point", "f32", 3)
CORNER_VERT = Column("faces", ".corner_vert", "corner", "i32", 1)
LOOP_START = Column("faces", "loop_start", "face", "i32", 1)
MASK = Column("mask", ".sculpt_mask", "point", "f32", 1)
GROUP = Column("group", ".sculpt_face_set", "face", "i32", 1)
HIDE = Column("hide", ".hide_poly", "face", "u8", 1)
# Surface paint as the color attribute `Color`: GigaSculpt sends its sRGB bytes, red first, one
# point each, and the add-on makes it the active and the render color.
COLOR = Column("color", "Color", "point", "u8", 4)

# Every form the active color goes back in, by Blender's domain and data type: copied as Blender
# keeps it, bytes or floats, and made into GigaSculpt's colors by the engine.
COLORS = {
    ("POINT", "BYTE_COLOR"): COLOR,
    ("POINT", "FLOAT_COLOR"): Column("color", "Color", "point", "f32", 4),
    ("CORNER", "BYTE_COLOR"): Column("color", "Color", "corner", "u8", 4),
    ("CORNER", "FLOAT_COLOR"): Column("color", "Color", "corner", "f32", 4),
}

TABLE = (POSITION, CORNER_VERT, LOOP_START, MASK, GROUP, HIDE, *COLORS.values())

# The sculpt columns: sent in the Mesh form when the part has them; one Blender leaves out reads
# as zeros in GigaSculpt.
SCULPT = (MASK, GROUP, HIDE)

# Each sculpt column's Blender attribute type and domain.
ATTRIBUTE = {
    MASK: ("FLOAT", "POINT"),
    GROUP: ("INT", "FACE"),
    HIDE: ("BOOLEAN", "FACE"),
}

# The numpy type of each file's elements, little-endian.
DTYPE = {"f32": "<f4", "i32": "<i4", "u8": "u1"}

# What Blender derives from `.hide_poly` (decision 0015: hidden when every face around is).
HIDE_VERT = ".hide_vert"
HIDE_EDGE = ".hide_edge"
# The base's edge creases in the Levels form.
CREASE = "crease_edge"

# The object properties that tie a Blender object to its part.
PART = "gigasculpt_part"
RANK = "gigasculpt_rank"
# Show Paint as a float the render setup can read: 1.0 while the paint shows, 0.0 while it is
# hidden (decision 0032, section 8's amendment).
PAINT = "gigasculpt_paint"


def row(column):
    """The table's row for a manifest column, or None if any of its fields disagree."""
    for c in TABLE:
        if (
            column.get("gs") == c.gs
            and column.get("blender") == c.blender
            and column.get("domain") == c.domain
            and column.get("type") == c.type
            and column.get("width") == c.width
        ):
            return c
    return None


def entry(c, file):
    """A manifest column for row `c` stored in `file`."""
    return {
        "gs": c.gs,
        "blender": c.blender,
        "domain": c.domain,
        "type": c.type,
        "width": c.width,
        "file": file,
    }
