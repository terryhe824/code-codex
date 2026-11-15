"""Local control for one owner-private macOS acceptance session."""
import json
import os
from pathlib import Path
import select
import socket
import stat
import subprocess

COMMANDS = ("status", "observe", "attach", "detach", "stop")
SOCKET = "control.sock"
MAX_COMMAND = 256
MAX_REPLY = 64 * 1024


def start_background(command, log):
    return subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=log, stderr=log,
                            start_new_session=True, close_fds=True)


def private_root(root):
    root = Path(os.path.abspath(root))
    if root != root.resolve(strict=True):
        raise ValueError("session directory must use its canonical path")
    metadata = root.lstat()
    if (not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != os.geteuid()
            or stat.S_IMODE(metadata.st_mode) != 0o700):
        raise ValueError("session directory must be owned by this user and mode 0700")
    return root


def _read_record(connection, limit):
    buffer = bytearray()
    while len(buffer) <= limit:
        chunk = connection.recv(min(1024, limit + 1 - len(buffer)))
        if not chunk:
            break
        buffer.extend(chunk)
        if b"\n" in buffer:
            if len(buffer) <= limit and buffer.count(b"\n") == 1 and buffer.endswith(b"\n"):
                try:
                    return json.loads(buffer)
                except (UnicodeError, ValueError, RecursionError):
                    break
            break
    raise ValueError("invalid control message")


def _send_record(connection, value):
    data = (json.dumps(value, separators=(",", ":")) + "\n").encode()
    if len(data) > MAX_REPLY:
        raise ValueError("controller response exceeds its limit")
    connection.sendall(data)


class ControlServer:
    def __init__(self, root, handle):
        self.path = private_root(root) / SOCKET
        if len(os.fsencode(self.path)) >= 104:
            raise ValueError("session path is too long for a macOS control socket")
        self.handle = handle
        self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.identity = None
        try:
            self.socket.bind(str(self.path))
            metadata = self.path.lstat()
            self.identity = (metadata.st_dev, metadata.st_ino)
            self.path.chmod(0o600)
            self.socket.listen(8)
            self.socket.setblocking(False)
        except BaseException:
            self.close()
            raise

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()

    def close(self):
        self.socket.close()
        try:
            metadata = self.path.lstat()
            if stat.S_ISSOCK(metadata.st_mode) and self.identity == (metadata.st_dev, metadata.st_ino):
                self.path.unlink()
        except FileNotFoundError:
            pass

    def serve_once(self, timeout):
        if not select.select([self.socket], [], [], timeout)[0]:
            return
        try:
            connection, _ = self.socket.accept()
        except BlockingIOError:
            return
        with connection:
            # One bounded request per connection. A disconnected/stalling
            # client cannot stop the host or strand future commands.
            connection.settimeout(0.3)
            try:
                value = _read_record(connection, MAX_COMMAND)
                if not isinstance(value, dict) or set(value) != {"command"} or value["command"] not in COMMANDS:
                    raise ValueError("unsupported control command")
            except (OSError, ValueError):
                reply = {"ok": False, "error": "invalid control request"}
            else:
                try:
                    state = self.handle(value["command"])
                except Exception as error:
                    reply = {"ok": False, "error": f"control action failed ({type(error).__name__})"}
                else:
                    reply = {"ok": True, "state": state}
            try:
                _send_record(connection, reply)
            except (OSError, ValueError):
                pass


def request(root, command):
    if command not in COMMANDS:
        raise ValueError("unsupported control command")
    path = private_root(root) / SOCKET
    metadata = path.lstat()
    if (not stat.S_ISSOCK(metadata.st_mode) or metadata.st_uid != os.geteuid()
            or stat.S_IMODE(metadata.st_mode) != 0o600):
        raise ValueError("control endpoint is not the private session socket")
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.settimeout(15)
        connection.connect(str(path))
        _send_record(connection, {"command": command})
        reply = _read_record(connection, MAX_REPLY)
    if not isinstance(reply, dict) or reply.get("ok") is not True or not isinstance(reply.get("state"), dict):
        raise RuntimeError(reply.get("error", "invalid controller response") if isinstance(reply, dict)
                           else "invalid controller response")
    return reply["state"]
