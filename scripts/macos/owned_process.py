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

    def exited(self):
        if not self._exited:
            self._exited = bool(self._events.control(None, 1, 0))
        return self._exited

    def close(self):
        try:
            child = self._child
            if child is None or child.returncode is not None:
                return
            group = child.pid
            if group <= 1 or group == os.getpgrp():
                raise RuntimeError("refusing to signal a process group without owned identity")
            # Use group existence, never leader liveness. Do not poll/wait
            # or otherwise reap the leader until the final group signal.
            try:
                os.killpg(group, signal.SIGTERM)
            except ProcessLookupError:
                pass
            else:
                deadline = time.monotonic() + 1
                while time.monotonic() < deadline:
                    try:
                        os.killpg(group, 0)
                    except ProcessLookupError:
                        break
                    time.sleep(0.05)
                try:
                    os.killpg(group, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            return child.wait(timeout=3)
        finally:
            self._events.close()
