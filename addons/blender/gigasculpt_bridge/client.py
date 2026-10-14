# SPDX-License-Identifier: GPL-3.0-or-later
"""A small client for a running GigaSculpt session: JSON-RPC 2.0, one JSON object per line,
over the session's Unix socket (ARCHITECTURE 7.2). The add-on is one more client with the agent
role; it runs commands with `cmd.run`, and `job.start` and `job.status` for a long call.

No bpy here, so it runs anywhere Python does.
"""

import json
import os
import socket

from .folder import ExchangeError, check_session

PROTOCOL = 1


def socket_path(session):
    """The socket of session `session`: `$XDG_RUNTIME_DIR/gigasculpt/<session>.sock`."""
    check_session(session)
    runtime = os.environ.get("XDG_RUNTIME_DIR")
    if not runtime:
        raise ExchangeError("XDG_RUNTIME_DIR is not set, so there is no GigaSculpt session to find")
    return os.path.join(runtime, "gigasculpt", f"{session}.sock")


class Client:
    """One connection to a session. `timeout` is how long to wait for any one reply, in
    seconds: a big part takes a while to write."""

    def __init__(self, path, name="blender", timeout=600.0):
        self.path = path
        try:
            self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            self.sock.settimeout(timeout)
            self.sock.connect(path)
        except OSError as e:
            raise ExchangeError(
                f"no GigaSculpt session answers at {path} ({e.strerror or e}). Start "
                "GigaSculpt, or check the session name."
            ) from None
        self.lines = self.sock.makefile("rb")
        self.next_id = 0
        self.hello = self.call(
            "engine.hello", {"client": name, "role": "agent", "protocol": PROTOCOL}
        )

    def close(self):
        self.lines.close()
        self.sock.close()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()

    def call(self, method, params):
        """Sends one request and returns its `result`; a protocol error raises."""
        self.next_id += 1
        line = json.dumps({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params})
        self.sock.sendall(line.encode() + b"\n")
        reply = self.lines.readline()
        if not reply:
            raise ExchangeError(f"the GigaSculpt session at {self.path} closed the connection")
        reply = json.loads(reply)
        if "error" in reply:
            e = reply["error"]
            raise ExchangeError(f"GigaSculpt refused {method}: {e.get('message')}")
        return reply["result"]

    def run(self, command, args):
        """Runs one command and returns its result; a command that fails raises with the
        engine's message and hint."""
        return outcome(command, self.call("cmd.run", {"command": command, "args": args}))

    def start(self, command, args, wait_ms=0):
        """Starts a command as a job (`job.start`) and returns its report."""
        return self.call("job.start", {"run": {"command": command, "args": args}, "wait_ms": wait_ms})

    def status(self, job, wait_ms=0):
        """A job's report (`job.status`), after waiting at most `wait_ms` for it to end."""
        return self.call("job.status", {"id": job, "wait_ms": wait_ms})

    def finish(self, command, args, wait_ms=1000):
        """Runs a command through `job.start`, waiting until it ends, and returns its result."""
        report = self.start(command, args, wait_ms)
        while "response" not in report:
            report = self.status(report["id"], wait_ms)
        return outcome(command, report["response"])


def outcome(command, response):
    """The result of a `cmd.run` response, or an error carrying the engine's message."""
    if "error" in response:
        e = response["error"]
        hint = f" ({e['hint']})" if e.get("hint") else ""
        raise ExchangeError(f"{command}: {e.get('message')}{hint}")
    return response["ok"]["result"]
