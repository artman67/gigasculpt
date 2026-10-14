# SPDX-License-Identifier: GPL-3.0-or-later
"""The add-on's folders against the engine's: for each case, a system (`sys.platform`'s name)
and an environment, the session and cache folders the add-on finds, `None` where it finds none.
The engine test compares them with gs_protocol's `session_dir_in` and `cache_dir_in`.

    blender -b ... --python folders_check.py -- <cases as JSON>
"""

import json
import os
import sys

sys.dont_write_bytecode = True  # no __pycache__ in the source tree
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import support  # noqa: E402

from gigasculpt_bridge import folder  # noqa: E402


def found(find, case):
    """What `find` gives for `case`, or `None` when it says there is no folder."""
    try:
        return find(case["platform"], case["env"])
    except folder.ExchangeError:
        return None


(cases,) = support.args(1)
support.result(
    [
        {"session": found(folder.session_dir, c), "cache": found(folder.cache_dir, c)}
        for c in json.loads(cases)
    ]
)
