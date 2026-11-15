"""Regression coverage for the lifecycle command's owned process cleanup."""
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

from test_lifecycle import run_owned_command


class LifecycleTimeoutTests(unittest.TestCase):
    def test_timeout_stops_descendant_that_ignores_term(self):
        with tempfile.TemporaryDirectory(prefix='code-codex-timeout-', dir='/private/tmp') as root:
            marker = Path(root) / 'fixture-pid'
            descendant = (
                'import os, signal, sys, time; from pathlib import Path; '
                'signal.signal(signal.SIGTERM, signal.SIG_IGN); '
                'Path(sys.argv[1]).write_text(str(os.getpid())); time.sleep(4)'
            )
            command = [
                sys.executable, '-c',
                'import subprocess, sys; subprocess.run(sys.argv[1:], check=True)',
                sys.executable, '-c', descendant, str(marker),
            ]
            with self.assertRaises(subprocess.TimeoutExpired):
                run_owned_command(command, timeout=1)
            pid = int(marker.read_text())
            deadline = time.monotonic() + 1
            while True:
                state = subprocess.run(
                    ['/bin/ps', '-o', 'stat=', '-p', str(pid)],
                    capture_output=True, text=True, check=False,
                ).stdout.strip()
                if not state or state.startswith('Z'):
                    break
                self.assertLess(time.monotonic(), deadline, 'fixture descendant survived timeout')
                time.sleep(0.05)


if __name__ == '__main__':
    unittest.main()
