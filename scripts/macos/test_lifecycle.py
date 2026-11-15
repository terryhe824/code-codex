#!/usr/bin/env python3
"""Exercise the native installer against a disposable signed installation."""
import argparse
from contextlib import contextmanager
import fcntl
import json
from pathlib import Path
import platform
import select
import subprocess
import sys
import tempfile

from owned_process import OwnedGroup


@contextmanager
def running_fixture(executable, env):
    with subprocess.Popen(
        [executable], env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE
    ) as process:
        try:
            ready, _, _ = select.select([process.stdout], [], [], 5)
            if not ready or process.stdout.readline() != b'ready\n':
                raise RuntimeError('The disposable executable did not start')
            yield
        finally:
            # The fixture has no descendants and exits when its stdin closes.
            process.stdin.close()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()


def check(condition, description):
    if not condition:
        raise AssertionError(description)
    print(f'PASS {description}', flush=True)


def run_owned_command(command, *, env=None, cwd=None, timeout=30, check=False):
    # File-backed output keeps the group leader waitable through teardown.
    with tempfile.TemporaryFile(mode='w+t', encoding='utf-8', errors='replace') as stdout, \
         tempfile.TemporaryFile(mode='w+t', encoding='utf-8', errors='replace') as stderr:
        group = OwnedGroup(
            command, env=env, cwd=cwd, stdin=subprocess.DEVNULL,
            stdout=stdout, stderr=stderr,
        )
        try:
            if not group.exited(timeout):
                raise subprocess.TimeoutExpired(command, timeout)
        finally:
            status = group.close()
        stdout.seek(0)
        stderr.seek(0)
        completed = subprocess.CompletedProcess(command, status, stdout.read(), stderr.read())
        if check:
            completed.check_returncode()
        return completed


def exercise(core, source, root):
    destination = root / '中文 安装' / 'Code-Codex.app'
    previous = destination.with_name('.Code-Codex.previous.app')
    home = root / 'isolated home'
    home.mkdir()
    # Restrict subprocesses to installer verbs that take explicit tmp paths.
    # Login registration and uninstall resolve the actual system user.
    env = {'HOME': str(home), 'TMPDIR': str(root), 'PATH': '/usr/bin:/bin'}

    def cli(*args):
        return run_owned_command([core, *args], env=env)

    def result(output):
        output.check_returncode()
        return json.loads(output.stdout)

    def digest(bundle):
        return result(cli('mac-package-digest', '--source', bundle))['sha256']

    def install(bundle, *options):
        return cli('mac-install', '--source', bundle, '--destination', destination, *options)

    def rollback():
        return cli('mac-rollback', '--destination', destination, '--allow-ad-hoc')

    original = digest(source)
    refused = install(source)
    check(refused.returncode == 25 and 'signing identity is not configured' in refused.stderr
          and not destination.exists(), 'release trust required without explicit local opt-in')
    installed = result(install(source, '--allow-ad-hoc', '--sha256', original))
    check(installed['installed'] and not installed['previousAvailable']
          and digest(destination) == original, 'signed artifact installs to a Unicode local path')

    fixture_code = root / 'fixture.c'
    fixture_code.write_text(
        '#include <stdio.h>\n'
        'int main(void) { puts("ready"); fflush(stdout); (void)getchar(); return 0; }\n'
    )
    executable = root / 'fixture'
    run_owned_command(
        ['/usr/bin/clang', '-arch', 'arm64', '-o', executable, fixture_code],
        env=env, check=True, timeout=30,
    )
    update = root / 'Synthetic update.app'
    run_owned_command(
        [sys.executable, '-c',
         'import sys; from package import create_bundle; create_bundle(*sys.argv[1:])',
         executable, update, '999.0.1', executable],
        env=env, cwd=Path(__file__).resolve().parent, check=True,
    )
    updated = digest(update)
    refused = install(update, '--allow-ad-hoc', '--sha256', '0' * 64)
    check(refused.returncode == 25 and 'SHA-256 does not match' in refused.stderr
          and digest(destination) == original and not previous.exists(),
          'bad update digest preserves the active bundle and previous state')

    with (destination.parent / '.code-codex-installation.lock').open('r+') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        refused = install(update, '--allow-ad-hoc')
        check(refused.returncode == 25 and digest(destination) == original
              and not previous.exists(), 'a competing installer lease prevents replacement')

    upgraded = result(install(update, '--allow-ad-hoc', '--sha256', updated))
    check(upgraded == {'installed': True, 'version': '999.0.1', 'previousAvailable': True}
          and digest(destination) == updated and digest(previous) == original,
          'upgrade publishes the new bundle and saves the previous bundle')
    for name in ('Code-Codex', 'code-codex-core'):
        with running_fixture(destination / 'Contents' / 'MacOS' / name, env):
            refused = rollback()
            check(refused.returncode == 25 and 'Quit the running Code-Codex' in refused.stderr
                  and digest(destination) == updated and digest(previous) == original,
                  f'rollback preserves both bundles while the installed {name} runs')

    rolled_back = result(rollback())
    check(rolled_back['rolledBack'] and rolled_back['version'] == installed['version']
          and digest(destination) == original and digest(previous) == updated,
          'rollback swaps active and previous bundles after the fixture exits')
    with running_fixture(previous / 'Contents/MacOS/Code-Codex', env):
        refused = install(source, '--allow-ad-hoc')
        check(refused.returncode == 25 and 'Quit the running Code-Codex' in refused.stderr
              and digest(destination) == original and digest(previous) == updated,
              'an upgrade cannot replace a running previous bundle')

    for app in (destination, previous):
        run_owned_command(['/usr/bin/codesign', '--verify', '--deep', '--strict', app],
                          env=env, check=True)
    check(True, 'both installed versions retain valid macOS signatures')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--core', required=True, type=Path, help='built Mac CLI')
    parser.add_argument('--source', required=True, type=Path, help='local ad-hoc signed app')
    args = parser.parse_args()
    if platform.system() != 'Darwin' or platform.machine() != 'arm64':
        parser.error('this acceptance test requires Apple Silicon macOS')
    with tempfile.TemporaryDirectory(prefix='code-codex-lifecycle-', dir='/private/tmp') as root:
        exercise(args.core.resolve(strict=True), args.source.resolve(strict=True), Path(root))


if __name__ == '__main__':
    main()
