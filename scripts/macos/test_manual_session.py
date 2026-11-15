"""Synthetic behavioral tests for manual session control. No app is opened."""
import json
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
import os
import runpy
import signal
import socket
import subprocess
import sys
import tempfile
from threading import Thread
import time
import unittest
from urllib.error import HTTPError

SCRIPTS = Path(__file__).resolve().parent
CLI = SCRIPTS / "manual-session.py"
FIXTURE = r"""
import json, sys
from pathlib import Path
sys.path.insert(0, sys.argv[1])
from manual_session_control import ControlServer
root = Path(sys.argv[2])
state = {'status':'awaiting-manual-login', 'attached':False}
def handle(command):
    if command == 'attach':
        state['attached'] = True
    if command == 'detach':
        state['attached'] = False
    if command == 'stop':
        state['status'] = 'closed'
    return state
with ControlServer(root, handle) as server:
    while state['status'] != 'closed':
        server.serve_once(0.1)
"""


class ManualControlTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(dir="/private/tmp")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name) / "fixture"
        self.root.mkdir(mode=0o700)

    def start(self):
        child = subprocess.Popen(
            [sys.executable, "-c", FIXTURE, str(SCRIPTS), str(self.root)],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
        )

        def cleanup():
            if child.poll() is None:
                child.terminate()
            child.wait(timeout=3)
            child.stderr.close()

        self.addCleanup(cleanup)
        deadline = time.monotonic() + 3
        while not (self.root / "control.sock").exists() and child.poll() is None and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertTrue((self.root / "control.sock").exists(), child.stderr.read().decode() if child.poll() is not None else "controller startup timed out")
        return child

    def invoke(self, command, *extra):
        return subprocess.run(
            [sys.executable, str(CLI), "--session", str(self.root), "--command", command, *extra],
            capture_output=True,
            text=True,
            timeout=4,
        )

    def state(self, command):
        result = self.invoke(command)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def test_background_launcher_exits_and_controller_remains_available(self):
        launcher = """
import sys
sys.path.insert(0, sys.argv[1])
from manual_session_control import start_background
with open(sys.argv[2] + '/controller.log', 'wb') as log:
    child = start_background([sys.executable, "-c", sys.argv[3], sys.argv[1], sys.argv[2]], log)
    print(child.pid)
"""
        created = subprocess.run(
            [sys.executable, "-c", launcher, str(SCRIPTS), str(self.root), FIXTURE],
            capture_output=True, text=True, check=True, timeout=3,
        )
        pid = int(created.stdout)
        deadline = time.monotonic() + 3
        while not (self.root / "control.sock").exists() and time.monotonic() < deadline:
            time.sleep(0.01)
        try:
            self.assertEqual(os.getsid(pid), pid)
            self.assertEqual(self.state("status")["status"], "awaiting-manual-login")
        finally:
            self.state("stop")

    def test_owned_supervisor_can_handle_sigint_before_cleanup(self):
        from owned_process import OwnedGroup
        marker = self.root / "interrupt"
        fixture = r"""
import signal, sys, time
from pathlib import Path
def interrupted(*args):
    Path(sys.argv[1]).write_text("handled")
    sys.exit(0)
signal.signal(signal.SIGINT, interrupted)
print("ready", flush=True)
while True:
    time.sleep(1)
"""
        with (self.root / "child.log").open("w+b") as log:
            child = OwnedGroup([sys.executable, "-c", fixture, str(marker)],
                               stdin=subprocess.DEVNULL, stdout=log, stderr=log)
            try:
                deadline = time.monotonic() + 2
                while (self.root / "child.log").stat().st_size == 0 and time.monotonic() < deadline:
                    time.sleep(0.01)
                self.assertEqual(child.close(initial_signal=signal.SIGINT, grace=1), 0)
                self.assertEqual(marker.read_text(), "handled")
            finally:
                child.close()

    def test_page_selection_requires_one_exact_canonical_page(self):
        select_page = runpy.run_path(str(CLI))["_canonical_page"]
        canonical = {"type": "page", "url": "app://-/index.html", "id": "main"}
        overlay = {"type": "page", "url": "app://-/index.html?initialRoute=overlay", "id": "overlay"}
        self.assertIs(select_page([overlay, canonical]), canonical)
        with self.assertRaisesRegex(RuntimeError, "exactly one"):
            select_page([overlay])
        with self.assertRaisesRegex(RuntimeError, "exactly one"):
            select_page([canonical, {**canonical, "id": "second"}])

    def stopped_session(self):
        existing = self.root / "logged-in"
        existing.mkdir(mode=0o700)
        for name in ("home", "profile", "codex", "workspace"):
            (existing / name).mkdir(mode=0o700)
        app = existing / "ChatGPT.app"
        app.mkdir(mode=0o700)
        receipt = existing / "session.json"
        dead = subprocess.Popen(["/usr/bin/true"])
        dead.wait(timeout=2)
        state = {"schema": 1, "status": "closed", "session": str(existing), "hostPid": dead.pid,
                 "home": str(existing / "home"), "codexHome": str(existing / "codex"),
                 "profile": str(existing / "profile"), "workspace": str(existing / "workspace")}
        receipt.write_text(json.dumps(state))
        receipt.chmod(0o600)
        return existing, app, state

    def test_resumed_run_reuses_only_private_profile_and_workspace(self):
        prepare = runpy.run_path(str(CLI))["_resumed_launch"]
        existing, app, state = self.stopped_session()
        receipt = existing / "session.json"
        workspace = existing / "workspace/Hello.txt"
        workspace.write_text("edited by the acceptance operator")
        command, env = prepare(existing, app, Path("/synthetic/code-codex"))
        self.assertEqual(command[1:4], ["run", "--codex-exe", str(app)])
        self.assertIn(f"--codex-arg=--user-data-dir={existing / 'profile'}", command)
        self.assertEqual(env["HOME"], str(existing / "home"))
        self.assertEqual(env["CODEX_HOME"], str(existing / "codex"))
        self.assertEqual(env["CODEX_SQLITE_HOME"], str(existing / "codex"))
        self.assertEqual(workspace.read_text(), "edited by the acceptance operator")
        with self.assertRaisesRegex(ValueError, "private session"):
            prepare(existing, Path("/Applications/ChatGPT.app"), Path("/synthetic/code-codex"))
        receipt.write_text(json.dumps({**state, "status": "observed"}))
        with self.assertRaisesRegex(ValueError, "closed"):
            prepare(existing, app, Path("/synthetic/code-codex"))
        receipt.write_text(json.dumps({**state, "hostPid": os.getpid()}))
        with self.assertRaisesRegex(ValueError, "still running"):
            prepare(existing, app, Path("/synthetic/code-codex"))
        receipt.write_text(json.dumps(state))
        (existing / "codex").rmdir()
        (existing / "codex").symlink_to(existing / "home")
        with self.assertRaisesRegex(ValueError, "canonical"):
            prepare(existing, app, Path("/synthetic/code-codex"))

    def resumed_fixture(self):
        core = self.root / "fixture-core"
        core.write_text(f"#!{sys.executable}\n" + r'''
import json, os, signal, sys, time
from pathlib import Path
home = Path(os.environ["HOME"])
def interrupt(*_):
    (home / "interrupted").touch()
    sys.exit(0)
signal.signal(signal.SIGINT, interrupt)
(home / "record.json").write_text(json.dumps(sys.argv[1:]))
while not (home / "release").exists():
    time.sleep(0.05)
''')
        core.chmod(0o700)
        invoke = """
import runpy, sys
from pathlib import Path
sys.path.insert(0, sys.argv[1])
module = runpy.run_path(str(Path(sys.argv[1]) / 'manual-session.py'))
module['_resumed_controller'](*map(Path, sys.argv[2:]))
"""
        return [sys.executable, "-c", invoke, str(SCRIPTS)], core

    def start_resumed_fixture(self, command, existing, app, core):
        controller = subprocess.Popen(command + [str(self.root), str(existing), str(app), str(core)],
                                      stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)

        def cleanup():
            if controller.poll() is None:
                controller.terminate()
            controller.wait(timeout=6)
            controller.stderr.close()

        self.addCleanup(cleanup)
        self.addCleanup((existing / "home/release").touch)
        record = existing / "home/record.json"
        deadline = time.monotonic() + 3
        while not record.exists() and controller.poll() is None and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertTrue(record.exists())
        return controller, record

    def test_resumed_controller_owns_core_and_releases_profile_after_stop(self):
        existing, app, _ = self.stopped_session()
        command, core = self.resumed_fixture()
        controller, record = self.start_resumed_fixture(command, existing, app, core)
        state = self.state("status")
        self.assertEqual(state["status"], "run-started")
        self.assertTrue(state["coreRunning"])
        self.assertEqual(os.getsid(state["corePid"]), state["corePid"])
        self.assertEqual(json.loads(record.read_text())[:3], ["run", "--codex-exe", str(app)])
        rival = self.root / "rival"
        rival.mkdir(mode=0o700)
        result = subprocess.run(command + [str(rival), str(existing), str(app), str(core)],
                                capture_output=True, text=True, timeout=2)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("another resumed run owns this private profile", result.stderr)
        self.assertEqual(self.state("stop")["status"], "stopping")
        self.assertEqual(controller.wait(timeout=6), 0)
        self.assertTrue((existing / "home/interrupted").exists())
        self.assertEqual(json.loads((self.root / "session.json").read_text())["coreExitCode"], 0)

    def test_resumed_core_blocks_profile_reuse_after_controller_crash(self):
        existing, app, _ = self.stopped_session()
        command, core = self.resumed_fixture()
        controller, _ = self.start_resumed_fixture(command, existing, app, core)
        own_core = self.state("status")["corePid"]
        controller.kill()
        controller.wait(timeout=2)
        self.assertEqual(os.getsid(own_core), own_core)
        rival = self.root / "rival"
        rival.mkdir(mode=0o700)
        fallback = subprocess.Popen(command + [str(rival), str(existing), str(app), str(core)],
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            _, error = fallback.communicate(timeout=1)
        except subprocess.TimeoutExpired:
            (existing / "home/release").touch()
            fallback.communicate(timeout=6)
            self.fail("a new controller launched while the previous core still owned the profile")
        self.assertNotEqual(fallback.returncode, 0)
        self.assertIn("a previous resumed run is still active", error)

    def test_resumed_core_never_runs_without_published_group(self):
        spawn = runpy.run_path(str(CLI))["_spawn_resumed_run"]
        marker = self.root / "unexpected-exec"

        def failed_publish(*_):
            raise OSError("fixture commit failure")

        spawn.__globals__["_record_resumed_group"] = failed_publish
        with (self.root / "log").open("xb") as log:
            with self.assertRaisesRegex(OSError, "fixture commit failure"):
                spawn(self.root, ["/usr/bin/touch", str(marker)], {}, log)
        self.assertFalse(marker.exists())

    def test_page_websocket_must_stay_on_the_verified_listener_and_target(self):
        endpoint = runpy.run_path(str(CLI))["_websocket_endpoint"]
        url = "ws://127.0.0.1:1234/devtools/page/current"
        target = {"id": "current", "webSocketDebuggerUrl": url}
        self.assertEqual(endpoint(target, 1234), url)
        for other in (url.replace(":1234", ":1235"), url.replace("127.0.0.1", "localhost"),
                      url.replace("/current", "/other"), url + "?extra=1"):
            with self.subTest(other=other), self.assertRaisesRegex(RuntimeError, "verified listener"):
                endpoint({**target, "webSocketDebuggerUrl": other}, 1234)

    def test_debug_target_discovery_does_not_follow_http_redirects(self):
        opener = runpy.run_path(str(CLI))["_debug_opener"]()
        requests = []

        class LocalHandler(BaseHTTPRequestHandler):
            def do_GET(self):
                requests.append(self.path)
                self.send_response(302 if self.path == "/json/list" else 200)
                self.send_header("Location", "/foreign-targets")
                self.end_headers()

            def log_message(self, *args):
                pass

        with HTTPServer(("127.0.0.1", 0), LocalHandler) as server:
            thread = Thread(target=server.serve_forever, daemon=True)
            thread.start()
            try:
                with self.assertRaises(HTTPError) as error:
                    opener.open(f"http://127.0.0.1:{server.server_port}/json/list", timeout=1)
                self.assertEqual(error.exception.code, 302)
                error.exception.close()
            finally:
                server.shutdown()
                thread.join(timeout=2)
        self.assertEqual(requests, ["/json/list"])

    def test_multiple_client_processes_control_one_session_without_stdin(self):
        child = self.start()
        self.assertEqual(self.state("status")["status"], "awaiting-manual-login")
        self.assertTrue(self.state("attach")["attached"])
        self.assertTrue(self.state("status")["attached"])
        self.assertFalse(self.state("detach")["attached"])
        self.assertEqual(self.state("stop")["status"], "closed")
        self.assertEqual(child.wait(timeout=3), 0)
        self.assertFalse((self.root / "control.sock").exists())
        self.assertNotEqual(self.invoke("status").returncode, 0)

    def test_invalid_request_has_no_effect_and_client_rejects_nonprivate_directory(self):
        self.start()
        with socket.socket(socket.AF_UNIX) as client:
            client.settimeout(2)
            client.connect(str(self.root / "control.sock"))
            client.sendall(b'{"command":"attach","core":"/tmp/arbitrary"}\n')
            reply = json.loads(client.makefile("rb").readline())
        self.assertFalse(reply["ok"])
        self.assertFalse(self.state("status")["attached"])
        self.assertNotEqual(self.invoke("attach", "--core", "/tmp/arbitrary").returncode, 0)
        self.root.chmod(0o755)
        self.assertNotEqual(self.invoke("status").returncode, 0)
        self.root.chmod(0o700)
        self.state("stop")

    def test_a_client_that_stalls_does_not_strand_control(self):
        self.start()
        with socket.socket(socket.AF_UNIX) as stalled:
            stalled.connect(str(self.root / "control.sock"))
            self.assertEqual(self.state("status")["status"], "awaiting-manual-login")
        self.state("stop")


if __name__ == "__main__":
    unittest.main()
