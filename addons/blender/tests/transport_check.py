# SPDX-License-Identifier: GPL-3.0-or-later
"""The add-on's client over a session socket on this system (decision 0063): it says hello and
runs one command against gs-protocol's test server, which echoes the command's arguments. On
Windows this is Winsock's Unix socket, which Python's own socket module does not offer.

    blender -b ... --python transport_check.py -- <socket>
"""

import os
import sys

sys.dont_write_bytecode = True  # no __pycache__ in the source tree
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import support  # noqa: E402

from gigasculpt_bridge import client  # noqa: E402

(path,) = support.args(1)
with client.Client(path, "blender-test") as c:
    echoed = c.run("test.echo", {"text": "héllo"})
    hello = c.hello
support.result({"hello": hello, "echoed": echoed, "platform": sys.platform})
