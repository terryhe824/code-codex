"""Owned macOS process groups for disposable host acceptance sessions."""
import os, select, signal, subprocess, time


class OwnedGroup:
    """Own a fresh session and keep its leader PID reserved through teardown."""

    def __init__(self, command, **kwargs):
        self._events = select.kqueue()
        self._exited = False
        self._child = None
        try:
            self._child = subprocess.Popen(command, start_new_session=True, **kwargs)
            try:
                self._events.control([
                    select.kevent(
                        self.pid,
                        filter=select.KQ_FILTER_PROC,
                        flags=select.KQ_EV_ADD | select.KQ_EV_ONESHOT,
                        fflags=select.KQ_NOTE_EXIT,
                    )
                ], 0, 0)
            except ProcessLookupError:
                # macOS stops exposing an exited child to kqueue even though
                # our waitable child still reserves its PID.
                self._exited = True
        except BaseException:
            self.close()
            raise

    @property
    def pid(self):
        return self._child.pid

    def exited(self, timeout=0):
        if not self._exited:
            self._exited = bool(self._events.control(None, 1, timeout))
        return self._exited

    def _signal_group(self, group, sig):
        try:
            os.killpg(group, sig)
            return True
        except ProcessLookupError:
            return False
        except PermissionError:
            # Darwin can return EPERM for a group containing only the
            # waitable group leader. Its PID is still reserved.
            if not self.exited():
                raise
            return False

    def close(self, *, initial_signal=signal.SIGTERM, grace=1):
        try:
            child = self._child
            if child is None or child.returncode is not None:
                return None if child is None else child.returncode
            group = child.pid
            if group <= 1 or group == os.getpgrp():
                raise RuntimeError("refusing to signal a process group without owned identity")
            # Use group existence, never leader liveness. Do not poll/wait
            # or otherwise reap the leader until the final group signal.
            if self._signal_group(group, initial_signal):
                deadline = time.monotonic() + grace
                while time.monotonic() < deadline and self._signal_group(group, 0):
                    time.sleep(0.05)
                self._signal_group(group, signal.SIGKILL)
            return child.wait(timeout=3)
        finally:
            self._events.close()
