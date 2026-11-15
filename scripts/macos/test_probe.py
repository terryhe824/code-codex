"""Run probe cleanup fixtures without starting or contacting the official app."""
import contextlib
import io
import os
import plistlib
from pathlib import Path
import runpy
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

PROBE = Path(__file__).with_name("probe-host.py")
FIXTURE = """
import os, signal, sys, time
pid_file, marker, mode = sys.argv[1:]
helper = os.fork()
if helper:
    while not os.path.exists(pid_file):
        time.sleep(0.005)
    if mode == 'early':
        os._exit(7)
    time.sleep(10)
signal.signal(signal.SIGTERM, signal.SIG_IGN)
signal.signal(signal.SIGINT, signal.SIG_IGN)
with open(pid_file, 'w') as out:
    out.write(str(os.getpid()))
time.sleep(5.3)
with open(marker, 'w') as out:
    out.write('survived')
"""


class ProbeCleanup(unittest.TestCase):
    def run_fixture(self, mode, *, report_failure=False):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp).resolve()
            report = root / "probe.json"
            app = root / "Fixture.app"
            core = root / "fixture-core"
            core.touch()
            owned = []
            indicators = {}
            real_spawn = subprocess.Popen

            def spawn(command, *args, **kwargs):
                if command[0] in (str(core), str(app / "Contents/MacOS/Fixture")):
                    role = "supervisor" if command[0] == str(core) else "host"
                    attempt = len(owned)
                    pid_file = root / f"{role}.{attempt}.pid"
                    marker = root / f"{role}.{attempt}.survived"
                    indicators[role] = (pid_file, marker)
                    fixture = real_spawn(
                        [sys.executable, "-c", FIXTURE, str(pid_file), str(marker),
                         mode if role == "host" else "early"],
                        *args, **kwargs
                    )
                    owned.append(fixture)
                    until = time.monotonic() + 2
                    while not pid_file.is_file():
                        if time.monotonic() >= until:
                            raise RuntimeError("helper setup timed out")
                        time.sleep(0.01)
                    if mode == "early" or role == "supervisor":
                        time.sleep(0.05)
                    return fixture
                return real_spawn(command, *args, **kwargs)

            def run(command, *args, **kwargs):
                if command[0] == "/usr/sbin/lsof":
                    if not report_failure:
                        raise RuntimeError("end fixture while host is live")
                    if len(owned) > 1:
                        raise RuntimeError("unexpected repeated page probe")
                    return subprocess.CompletedProcess(command, 0, stdout=f"p{owned[0].pid}\n")
                output = "" if command[0] == "/bin/ps" else '{"explorerMounted":false}'
                return subprocess.CompletedProcess(command, 0, stdout=output)

            def urlopen(url, **kwargs):
                body = '[{"type":"page","url":"app://-/index.html","webSocketDebuggerUrl":"ws://127.0.0.1"}]' if url.endswith("/list") else '{}'
                return io.StringIO(body)

            (app / "Contents").mkdir(parents=True)
            (app / "Contents/Info.plist").write_bytes(plistlib.dumps({
                "CFBundleExecutable": "Fixture", "CFBundleIdentifier": "fixture",
                "CFBundleShortVersionString": "fixture",
            }))
            if report_failure:
                (root / "probe.supervisor.log").mkdir()
            argv = [str(PROBE), "--app", str(app), "--report", str(report)]
            if report_failure:
                argv += ["--core", str(core)]
            try:
                with contextlib.ExitStack() as mocks:
                    mocks.enter_context(patch.object(sys, "argv", argv))
                    mocks.enter_context(patch.object(subprocess, "Popen", spawn))
                    mocks.enter_context(patch.object(subprocess, "run", run))
                    mocks.enter_context(patch("urllib.request.urlopen", urlopen))
                    mocks.enter_context(patch("urllib.request.install_opener"))
                    mocks.enter_context(contextlib.redirect_stdout(io.StringIO()))
                    if report_failure:
                        with self.assertRaisesRegex(OSError, "probe.supervisor.log"):
                            runpy.run_path(str(PROBE), run_name="__main__")
                    elif mode == "early":
                        runpy.run_path(str(PROBE), run_name="__main__")
                    else:
                        with self.assertRaisesRegex(RuntimeError, "end fixture"):
                            runpy.run_path(str(PROBE), run_name="__main__")
                self.assertEqual(set(indicators), {"host", "supervisor"} if report_failure else {"host"})
                for role, (pid_file, marker) in indicators.items():
                    pid = int(pid_file.read_text())
                    until = time.monotonic() + 6
                    while time.monotonic() < until and not marker.exists():
                        try:
                            os.kill(pid, 0)
                        except ProcessLookupError:
                            break
                        time.sleep(0.02)
                    self.assertFalse(marker.exists(), f"{role} left helper running")
                    with self.assertRaises(ProcessLookupError):
                        os.kill(pid, 0)
            finally:
                for process in owned:
                    if process.returncode is None:
                        # The fixture retains an unreaped Popen too.
                        try:
                            os.killpg(process.pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                        process.wait(timeout=2)

    def test_early_exit_still_cleans_process_group(self):
        self.run_fixture("early")

    def test_term_ignoring_helper_is_killed_after_leader_exits(self):
        self.run_fixture("term")

    def test_report_failure_does_not_skip_either_group(self):
        self.run_fixture("term", report_failure=True)


if __name__ == "__main__":
    unittest.main()
