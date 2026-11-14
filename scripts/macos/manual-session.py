#!/usr/bin/env python3
"""Keep an isolated official host open for manual login and explicit acceptance commands."""
import argparse
import json
import os
from pathlib import Path
import plistlib
import selectors
import socket
import subprocess
import sys
import time
import urllib.request

from owned_process import OwnedGroup


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--session', type=Path, required=True, help='New private session directory')
    parser.add_argument('--app', type=Path, default=Path('/Applications/ChatGPT.app'))
    parser.add_argument('--core', type=Path, required=True)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[2]
    app = args.app.resolve(strict=True)
    core = args.core.resolve(strict=True)
    subprocess.run(['/usr/bin/codesign', '--verify', '--strict', '--deep', '-R',
                    '=identifier "com.openai.codex" and anchor apple generic and certificate leaf[subject.OU] = "2DC432GLL2"',
                    str(app)], check=True, capture_output=True)
    info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
    executable = app / 'Contents/MacOS' / info['CFBundleExecutable']
    args.session.mkdir(mode=0o700, parents=True, exist_ok=False)
    root = args.session.resolve(strict=True)
    home, profile, codex, workspace = [root / name for name in ('home', 'profile', 'codex', 'workspace')]
    for path in (home, profile, codex, workspace, root / 'logs'):
        path.mkdir(mode=0o700)
    (workspace / 'Hello.txt').write_text('Code-Codex isolated Mac acceptance workspace.\n')
    (workspace / 'Preview.md').write_text('# Mac acceptance\n\nA **synthetic** preview fixture.\n')
    (workspace / 'Table.csv').write_text('name,value\nalpha,1\nbeta,2\n')
    env = {key: value for key, value in os.environ.items()
           if key in ('PATH', 'SHELL', 'USER', 'LOGNAME', 'LANG', 'LC_ALL', '__CF_USER_TEXT_ENCODING')}
    env.update(HOME=str(home), CODEX_HOME=str(codex), CODEX_SQLITE_HOME=str(codex),
               TMPDIR='/private/tmp', XDG_CONFIG_HOME=str(home / '.config'),
               XDG_DATA_HOME=str(home / '.local/share'), XDG_CACHE_HOME=str(home / '.cache'))
    with socket.socket() as reservation:
        reservation.bind(('127.0.0.1', 0))
        port = reservation.getsockname()[1]
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    state = {'schema': 1, 'status': 'starting', 'session': str(root), 'port': port,
             'home': str(home), 'codexHome': str(codex), 'profile': str(profile),
             'workspace': str(workspace), 'appVersion': info['CFBundleShortVersionString'],
             'keychainScope': 'same OS user; separate application data is not a separate keychain'}

    def publish(status, **fields):
        state.update(status=status, **fields)
        target = root / 'session.json'
        target.write_text(json.dumps(state, indent=2) + '\n')
        target.chmod(0o600)
        print(json.dumps(state), flush=True)

    def page():
        owners = subprocess.run(['/usr/sbin/lsof', '-nP', '-a', '-iTCP:' + str(port),
                                 '-sTCP:LISTEN', '-Fp'], capture_output=True, text=True, check=False)
        pids = [int(row[1:]) for row in owners.stdout.splitlines() if row.startswith('p')]
        if not pids or any(os.getpgid(pid) != host.pid for pid in pids):
            raise RuntimeError('test listener ownership not verified')
        with opener.open('http://127.0.0.1:' + str(port) + '/json/list', timeout=2) as response:
            targets = json.load(response)
        return next(target for target in targets
                    if target.get('type') == 'page' and target.get('url', '').split('?')[0] == 'app://-/index.html')

    host = supervisor = None
    with (root / 'logs/host.log').open('wb') as host_log, (root / 'logs/core.log').open('wb') as core_log:
        try:
            host = OwnedGroup([str(executable), '--user-data-dir=' + str(profile),
                               '--remote-debugging-address=127.0.0.1', '--remote-debugging-port=' + str(port),
                               '--no-first-run'], env=env, stdin=subprocess.DEVNULL,
                              stdout=host_log, stderr=host_log)
            state['hostPid'] = host.pid
            deadline = time.monotonic() + 40
            while time.monotonic() < deadline and not host.exited():
                try:
                    target = page()
                    break
                except (OSError, RuntimeError, StopIteration, ValueError):
                    time.sleep(0.25)
            else:
                raise RuntimeError('isolated test page did not become available')
            subprocess.run(['node', str(repo / 'scripts/macos/session-page.mjs')],
                           input=json.dumps({'endpoint': target['webSocketDebuggerUrl'], 'mark': True}),
                           text=True, capture_output=True, check=True, timeout=8)
            # AppKit activation targets this PID, not every app with the same bundle ID.
            activation = "ObjC.import('AppKit'); function run(a) { $.NSRunningApplication.runningApplicationWithProcessIdentifier(Number(a[0])).activateWithOptions(3); }"
            subprocess.run(['/usr/bin/osascript', '-l', 'JavaScript', '-', str(host.pid)],
                           input=activation, text=True, capture_output=True, timeout=5, check=False)
            publish('awaiting-manual-login')
            with selectors.DefaultSelector() as selector:
                selector.register(sys.stdin, selectors.EVENT_READ)
                while not host.exited():
                    if supervisor is not None and supervisor.exited():
                        code = supervisor.close()
                        supervisor = None
                        publish('core-ended', coreExitCode=code)
                    if not selector.select(timeout=0.5):
                        continue
                    command = sys.stdin.readline().strip()
                    if command in ('stop', ''):
                        break
                    if command == 'attach':
                        page()  # Recheck owned listener immediately before attaching.
                        if supervisor is None:
                            supervisor = OwnedGroup([str(core), 'attach', '--port', str(port),
                                                      '--workspace', str(workspace), '--allow-unsupported-version'],
                                                     env=env, stdin=subprocess.DEVNULL,
                                                     stdout=core_log, stderr=core_log)
                            publish('core-started', corePid=supervisor.pid)
                    elif command == 'status':
                        target = page()
                        result = subprocess.run(['node', str(repo / 'scripts/macos/session-page.mjs')],
                                                input=json.dumps({'endpoint': target['webSocketDebuggerUrl']}),
                                                text=True, capture_output=True, timeout=8, check=True)
                        publish('observed', page=json.loads(result.stdout))
                    elif command == 'detach' and supervisor is not None:
                        code = supervisor.close()
                        supervisor = None
                        publish('detached', coreExitCode=code)
        finally:
            try:
                if supervisor is not None:
                    supervisor.close()
            finally:
                if host is not None:
                    host.close()
                publish('closed')


if __name__ == '__main__':
    main()
