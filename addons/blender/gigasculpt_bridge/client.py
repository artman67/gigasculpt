# SPDX-License-Identifier: GPL-3.0-or-later
"""A small client for a running GigaSculpt session: JSON-RPC 2.0, one JSON object per line,
over the session's Unix socket (ARCHITECTURE 7.2), on Windows too (decision 0063). The add-on is
one more client with the agent role; it runs commands with `cmd.run`, and `job.start` and
`job.status` for a long call.

No bpy here, so it runs anywhere Python does.
"""

import json
import os
import socket

from .folder import ExchangeError, check_session, session_dir

PROTOCOL = 1
AF_UNIX = 1  # the same number on Linux, macOS and Windows


def socket_path(session):
    """The socket of session `session`: `<session>.sock` in the session folder."""
    check_session(session)
    return os.path.join(session_dir(), f"{session}.sock")


def connect(path, timeout):
    """A stream socket connected to the Unix socket at `path`, waiting at most `timeout` seconds
    for anything. Windows has had Unix sockets since Windows 10 version 1803, but Python's socket
    module does not offer them there, so Winsock makes and connects one, with no time limit on
    the connect, and Python's socket takes it over."""
    if hasattr(socket, "AF_UNIX"):
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.settimeout(timeout)
        try:
            sock.connect(path)
        except OSError:
            sock.close()
            raise
        return sock
    sock = socket.socket(fileno=_winsock_connect(path))
    sock.settimeout(timeout)
    return sock


def _winsock_connect(path):
    """The handle of a Winsock Unix socket connected to `path`, which Winsock takes as UTF-8."""
    import ctypes

    handle = ctypes.c_size_t  # SOCKET
    ws2 = ctypes.WinDLL("ws2_32", use_last_error=True)
    ws2.socket.argtypes, ws2.socket.restype = (ctypes.c_int,) * 3, handle
    ws2.connect.argtypes = (handle, ctypes.c_void_p, ctypes.c_int)
    ws2.closesocket.argtypes = (handle,)

    class Address(ctypes.Structure):
        _fields_ = (("family", ctypes.c_ushort), ("path", ctypes.c_char * 108))

    raw = os.fspath(path).encode("utf-8")
    if len(raw) >= 108:
        raise OSError(f"the socket path is {len(raw)} bytes, over Windows' 107")
    sock = ws2.socket(AF_UNIX, socket.SOCK_STREAM, 0)
    if sock == handle(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    address = Address(AF_UNIX, raw)
    if ws2.connect(sock, ctypes.byref(address), ctypes.sizeof(address)) != 0:
        error = ctypes.get_last_error()
        ws2.closesocket(sock)
        raise ctypes.WinError(error)
    return sock


class Client:
    """One connection to a session. `timeout` is how long to wait for any one reply, in
    seconds: a big part takes a while to write."""

    def __init__(self, path, name="blender", timeout=600.0):
        self.path = path
        try:
            self.sock = connect(path, timeout)
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
