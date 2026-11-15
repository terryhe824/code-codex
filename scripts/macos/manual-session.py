#!/usr/bin/env python3
"""Keep an isolated official host open for manual login and explicit acceptance commands."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import plistlib
import re
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import time
import urllib.request

from owned_process import OwnedGroup
from manual_session_control import COMMANDS, MAX_REPLY, SOCKET, ControlServer, private_root, request, start_background


def _create_session(path):
    path.mkdir(mode=0o700, parents=True, exist_ok=False)
    root = private_root(path.resolve(strict=True))
    if len(os.fsencode(root / SOCKET)) >= 104:
        root.rmdir()
        raise ValueError("session path is too long for a macOS control socket")
    return root


def _launch_background(root, app, core, reuse_profile_from):
    with (root / "controller.log").open("xb") as log:
        command = [sys.executable, str(Path(__file__).resolve()), "--session", str(root),
                   "--app", str(app), "--core", str(core), "--controller-parent", str(os.getpid())]
        if reuse_profile_from is not None:
            command.extend(["--reuse-profile-from", str(reuse_profile_from)])
        process = start_background(command, log)
        print(json.dumps({"schema": 1, "status": "starting", "session": str(root),
                          "controllerPid": process.pid, "control": str(root / SOCKET)}))


def _controller_session(path, parent):
    root = private_root(path)
    # The detached controller always owns a newly reserved control directory.
    # Any profile reuse is validated separately.
    if parent <= 1 or os.getppid() != parent and os.getppid() != 1:
        raise ValueError("controller handoff parent changed")
    if os.getsid(0) != os.getpid() or sorted(item.name for item in root.iterdir()) != ["controller.log"]:
        raise ValueError("controller handoff requires a new private session")
    return root


def _canonical_page(targets):
    if not isinstance(targets, list):
        raise RuntimeError('invalid target list from the verified listener')
    pages = [target for target in targets if isinstance(target, dict)
             and target.get('type') == 'page' and target.get('url') == 'app://-/index.html']
    if len(pages) != 1:
        raise RuntimeError('exactly one official test page is required')
    return pages[0]


def _websocket_endpoint(target, port):
    target_id = target.get('id')
    if not isinstance(target_id, str) or not re.fullmatch(r'[a-zA-Z0-9_-]{1,128}', target_id):
        raise RuntimeError('invalid target ID from the verified listener')
    expected = f'ws://127.0.0.1:{port}/devtools/page/{target_id}'
    if target.get('webSocketDebuggerUrl') != expected:
        raise RuntimeError('page endpoint does not match the verified listener')
    return expected


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def _debug_opener():
    return urllib.request.build_opener(urllib.request.ProxyHandler({}), _NoRedirect())


def _isolated_env(home, codex):
    env = {key: value for key, value in os.environ.items()
           if key in ('PATH', 'SHELL', 'USER', 'LOGNAME', 'LANG', 'LC_ALL', '__CF_USER_TEXT_ENCODING')}
    env.update(HOME=str(home), CODEX_HOME=str(codex), CODEX_SQLITE_HOME=str(codex),
               TMPDIR='/private/tmp', XDG_CONFIG_HOME=str(home / '.config'),
               XDG_DATA_HOME=str(home / '.local/share'), XDG_CACHE_HOME=str(home / '.cache'))
    return env


def _require_exited(pid):
    if type(pid) is not int or not 1 < pid <= 2**31 - 1:
        raise ValueError('invalid prior process in session receipt')
    for check in (os.kill, os.killpg):
        try:
            check(pid, 0)
        except ProcessLookupError:
            continue
        raise ValueError('a recorded prior test process or group is still running')


def _read_private_json(path):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, 'rb') as file:
        metadata = os.fstat(file.fileno())
        if (not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.geteuid()
                or stat.S_IMODE(metadata.st_mode) != 0o600 or metadata.st_size > MAX_REPLY):
            raise ValueError('invalid private session record')
        saved = file.read(MAX_REPLY + 1)
    if len(saved) > MAX_REPLY:
        raise ValueError('invalid private session record')
    state = json.loads(saved)
    if not isinstance(state, dict):
        raise ValueError('invalid private session record')
    return state


def _resumed_launch(source, app, core):
    source = private_root(source)
    if app.suffix != '.app' or not app.is_relative_to(source) or app != app.resolve(strict=True):
        raise ValueError('the signed app copy must be inside the prior private session')
    if os.path.lexists(source / SOCKET):
        raise ValueError('stop the prior test session controller before reusing its profile')
    state = _read_private_json(source / 'session.json')
    if state.get('schema') != 1 or state.get('session') != str(source):
        raise ValueError('invalid prior private session receipt')
    if state.get('status') != 'closed':
        raise ValueError('the prior private session must have a closed receipt')
    _require_exited(state.get('hostPid'))
    if state.get('corePid') is not None:
        _require_exited(state['corePid'])
    home, profile, codex, workspace = [private_root(source / name)
                                       for name in ('home', 'profile', 'codex', 'workspace')]
    expected_paths = {'home': home, 'profile': profile, 'codexHome': codex, 'workspace': workspace}
    if any(state.get(key) != str(value) for key, value in expected_paths.items()):
        raise ValueError('prior session receipt does not match the private directories')
    command = [str(core), 'run', '--codex-exe', str(app), '--workspace', str(workspace),
               '--codex-arg=--user-data-dir=' + str(profile), '--codex-arg=--no-first-run',
               '--allow-unsupported-version']
    return command, _isolated_env(home, codex)


def _check_previous_resume(source):
    try:
        prior = _read_private_json(source / 'resumed-run.json')
    except FileNotFoundError:
        return
    controller, group = prior.get('controllerPid'), prior.get('coreGroup')
    if (prior.get('schema') != 1 or type(controller) is not int or not 1 < controller <= 2**31 - 1
            or type(group) is not int or not 1 < group <= 2**31 - 1):
        raise ValueError('invalid previous resumed run record')
    for check, identity in ((os.kill, controller), (os.killpg, group)):
        try:
            check(identity, 0)
        except ProcessLookupError:
            continue
        raise ValueError('a previous resumed run is still active')


def _record_resumed_group(source, group):
    with tempfile.NamedTemporaryFile(mode='w', dir=source, prefix='.resumed-run-', delete=False) as file:
        staged = Path(file.name)
        try:
            json.dump({'schema': 1, 'controllerPid': os.getpid(), 'coreGroup': group}, file)
            file.flush()
            os.fsync(file.fileno())
            os.replace(staged, source / 'resumed-run.json')
        finally:
            staged.unlink(missing_ok=True)


def _spawn_resumed_run(source, command, env, log):
    # EOF before publication causes the gate process to exit. Once released,
    # exec preserves the recorded leader PID and its private process group.
    gate = ("import os,sys\n"
            "fd=int(sys.argv[1]); start=os.read(fd,1); os.close(fd)\n"
            "if start == b'1': os.execve(sys.argv[2],sys.argv[2:],os.environ)\n")
    read_fd, write_fd = os.pipe()
    run = None
    try:
        with os.fdopen(write_fd, 'wb', buffering=0) as release:
            run = OwnedGroup([sys.executable, '-c', gate, str(read_fd), *command], env=env,
                             pass_fds=(read_fd,), stdin=subprocess.DEVNULL, stdout=log, stderr=log)
            _record_resumed_group(source, run.pid)
            release.write(b'1')
        return run
    except BaseException:
        if run is not None:
            # Closing the pipe tells the unreleased gate to exit naturally.
            run.exited(1)
            run.close()
        raise
    finally:
        os.close(read_fd)


def _resumed_controller(root, source, app, core):
    # Serialize resumed controllers independently of their new control directories.
    fd = os.open(source / 'resumed-run.lock', os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600)
    with os.fdopen(fd, 'rb+') as lease:
        metadata = os.fstat(lease.fileno())
        if (not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.geteuid()
                or stat.S_IMODE(metadata.st_mode) != 0o600):
            raise ValueError('invalid private profile lease')
        try:
            fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise ValueError('another resumed run owns this private profile') from error
        command, env = _resumed_launch(source, app, core)
        _check_previous_resume(source)
        stopping = False
        run = None
        state = {'schema': 1, 'session': str(root), 'profileSession': str(source),
                 'controllerPid': os.getpid(), 'control': str(root / SOCKET), 'status': 'starting'}

        def stop(*_):
            nonlocal stopping
            stopping = True

        def handle(action):
            if action == 'stop':
                stop()
                state['status'] = 'stopping'
            elif action != 'status':
                raise ValueError('resumed runs accept status and stop only')
            return {**state, 'coreRunning': run is not None and not run.exited()}

        signal.signal(signal.SIGTERM, stop)
        signal.signal(signal.SIGINT, stop)
        with (root / 'core.log').open('xb') as log, ControlServer(root, handle) as controller:
            try:
                if not stopping:
                    run = _spawn_resumed_run(source, command, env, log)
                    state.update(status='run-started', corePid=run.pid)
                    _publish(root, state)
                while not stopping and run is not None and not run.exited():
                    controller.serve_once(0.5)
            finally:
                exit_code = run.close(initial_signal=signal.SIGINT, grace=3) if run is not None else None
                state.update(status='closed', corePid=None, coreExitCode=exit_code)
                _publish(root, state)


def _publish(root, state):
    target = root / 'session.json'
    target.write_text(json.dumps(state, indent=2) + '\n')
    target.chmod(0o600)
    try:
        print(json.dumps(state), flush=True)
    except BrokenPipeError:
        pass


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--session', type=Path, required=True, help='Private session directory')
    parser.add_argument('--app', type=Path)
    parser.add_argument('--core', type=Path)
    parser.add_argument('--command', choices=COMMANDS, help='Control an existing session')
    parser.add_argument('--background', action='store_true', help='Start a detached controller')
    parser.add_argument('--reuse-profile-from', type=Path, help='Run the signed app copy with a stopped private session profile')
    parser.add_argument('--controller-parent', type=int, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.command:
        if args.app is not None or args.core is not None or args.background or args.controller_parent is not None or args.reuse_profile_from is not None:
            parser.error('launch options are only valid when starting a new session')
        try:
            result = request(args.session, args.command)
        except (OSError, RuntimeError, ValueError) as error:
            parser.exit(1, f"manual-session control: {error}\n")
        print(json.dumps(result))
        return
    if args.core is None:
        parser.error('--core is required when starting a new session')
    repo = Path(__file__).resolve().parents[2]
    app = (args.app or Path('/Applications/ChatGPT.app')).resolve(strict=True)
    core = args.core.resolve(strict=True)
    if args.reuse_profile_from is not None:
        if args.app is None:
            parser.error('--reuse-profile-from requires an explicit signed app copy')
        _resumed_launch(args.reuse_profile_from, app, core)
    if args.background:
        if args.controller_parent is not None:
            parser.error('--background is only valid in the launcher')
        _launch_background(_create_session(args.session), app, core, args.reuse_profile_from)
        return
    root = (_controller_session(args.session, args.controller_parent) if args.controller_parent is not None
            else _create_session(args.session))
    subprocess.run(['/usr/bin/codesign', '--verify', '--strict', '--deep', '-R',
                    '=identifier "com.openai.codex" and anchor apple generic and certificate leaf[subject.OU] = "2DC432GLL2"',
                    str(app)], check=True, capture_output=True)
    if args.reuse_profile_from is not None:
        _resumed_controller(root, private_root(args.reuse_profile_from), app, core)
        return
    info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
    executable = app / 'Contents/MacOS' / info['CFBundleExecutable']
    home, profile, codex, workspace = [root / name for name in ('home', 'profile', 'codex', 'workspace')]
    for path in (home, profile, codex, workspace, root / 'logs'):
        path.mkdir(mode=0o700)
    (workspace / 'Hello.txt').write_text('Code-Codex isolated Mac acceptance workspace.\n')
    (workspace / 'Preview.md').write_text('# Mac acceptance\n\nA **synthetic** preview fixture.\n')
    (workspace / 'Table.csv').write_text('name,value\nalpha,1\nbeta,2\n')
    env = _isolated_env(home, codex)
    with socket.socket() as reservation:
        reservation.bind(('127.0.0.1', 0))
        port = reservation.getsockname()[1]
    opener = _debug_opener()
    state = {'schema': 1, 'status': 'starting', 'session': str(root), 'port': port,
             'home': str(home), 'codexHome': str(codex), 'profile': str(profile),
             'workspace': str(workspace), 'appVersion': info['CFBundleShortVersionString'],
             'controllerPid': os.getpid(), 'control': str(root / SOCKET),
             'keychainScope': 'same OS user; separate application data is not a separate keychain'}

    def publish(status, **fields):
        state.update(status=status, **fields)
        _publish(root, state)

    def page():
        try:
            owners = subprocess.run(['/usr/sbin/lsof', '-nP', '-a', '-iTCP:' + str(port),
                                     '-sTCP:LISTEN', '-Fp'], capture_output=True, text=True,
                                    check=True, timeout=2)
        except subprocess.SubprocessError as error:
            raise RuntimeError('test listener ownership not verified') from error
        pids = [int(row[1:]) for row in owners.stdout.splitlines() if row.startswith('p')]
        if not pids or any(os.getpgid(pid) != host.pid for pid in pids):
            raise RuntimeError('test listener ownership not verified')
        with opener.open('http://127.0.0.1:' + str(port) + '/json/list', timeout=2) as response:
            targets = json.load(response)
        return _websocket_endpoint(_canonical_page(targets), port)

    stopping = False

    def stop_requested(signum, frame):
        nonlocal stopping
        stopping = True

    signal.signal(signal.SIGTERM, stop_requested)
    signal.signal(signal.SIGINT, stop_requested)
    host = supervisor = None
    with (root / 'logs/host.log').open('wb') as host_log, (root / 'logs/core.log').open('wb') as core_log:
        try:
            host = OwnedGroup([str(executable), '--user-data-dir=' + str(profile),
                               '--remote-debugging-address=127.0.0.1', '--remote-debugging-port=' + str(port),
                               '--no-first-run'], env=env, stdin=subprocess.DEVNULL,
                              stdout=host_log, stderr=host_log)
            state['hostPid'] = host.pid
            publish('starting')
            deadline = time.monotonic() + 40
            while not stopping and time.monotonic() < deadline and not host.exited():
                try:
                    endpoint = page()
                    break
                except (OSError, RuntimeError, ValueError):
                    time.sleep(0.25)
            else:
                raise RuntimeError('isolated test page did not become available')
            subprocess.run(['node', str(repo / 'scripts/macos/session-page.mjs')],
                           input=json.dumps({'endpoint': endpoint, 'mark': True}),
                           text=True, capture_output=True, check=True, timeout=8)
            # AppKit activation targets this PID, not every app with the same bundle ID.
            activation = "ObjC.import('AppKit'); function run(a) { $.NSRunningApplication.runningApplicationWithProcessIdentifier(Number(a[0])).activateWithOptions(3); }"
            subprocess.run(['/usr/bin/osascript', '-l', 'JavaScript', '-', str(host.pid)],
                           input=activation, text=True, capture_output=True, timeout=5, check=False)
            def handle(command):
                nonlocal supervisor, stopping
                if command == 'stop':
                    stopping = True
                    publish('stopping')
                elif command == 'attach' and supervisor is None:
                    page()
                    supervisor = OwnedGroup([str(core), 'attach', '--port', str(port),
                                             '--workspace', str(workspace), '--allow-unsupported-version'],
                                            env=env, stdin=subprocess.DEVNULL,
                                            stdout=core_log, stderr=core_log)
                    publish('core-started', corePid=supervisor.pid, coreExitCode=None)
                elif command == 'observe':
                    endpoint = page()
                    result = subprocess.run(['node', str(repo / 'scripts/macos/session-page.mjs')],
                                            input=json.dumps({'endpoint': endpoint}),
                                            text=True, capture_output=True, timeout=8, check=True)
                    publish('observed', page=json.loads(result.stdout))
                elif command == 'detach' and supervisor is not None:
                    code = supervisor.close(initial_signal=signal.SIGINT, grace=3)
                    supervisor = None
                    publish('detached', coreExitCode=code, corePid=None)
                return {**state, 'hostRunning': not host.exited(),
                        'coreRunning': supervisor is not None and not supervisor.exited()}

            with ControlServer(root, handle) as controller:
                publish('awaiting-manual-login')
                while not stopping and not host.exited():
                    if supervisor is not None and supervisor.exited():
                        code = supervisor.close(initial_signal=signal.SIGINT, grace=3)
                        supervisor = None
                        publish('core-ended', coreExitCode=code, corePid=None)
                    controller.serve_once(0.5)
        finally:
            try:
                if supervisor is not None:
                    supervisor.close(initial_signal=signal.SIGINT, grace=3)
            finally:
                if host is not None:
                    host.close()
            publish('closed', corePid=None)


if __name__ == '__main__':
    main()
