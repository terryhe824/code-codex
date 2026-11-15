import contextlib
import hashlib
import io
import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import package


SCRIPTS = Path(__file__).resolve().parent
VERSION = '0.6.20'
TEAM = 'AB12CD34EF'
IDENTITY = 'D' * 40
JOB = '2efe2717-52ef-43a5-96dc-0797e4ca1041'
APP_CDHASH = 'a' * 40
CORE_CDHASH = 'b' * 40


def cli(*args):
    return subprocess.run([sys.executable, str(SCRIPTS/'release.py'), *map(str, args)],
                          capture_output=True, text=True, timeout=180)


class ReleaseTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        temp = tempfile.TemporaryDirectory(prefix='code-codex-release-test-', dir='/private/tmp')
        cls.addClassCleanup(temp.cleanup)
        cls.fixtures = Path(temp.name)
        source = cls.fixtures/'test.c'
        source.write_text('int main(void) { return 0; }\n')
        for label, architecture, minimum in [('supported', 'arm64', '13.0'),
                                              ('newer', 'arm64', '14.0'),
                                              ('intel', 'x86_64', '13.0')]:
            subprocess.run(['/usr/bin/clang', '-arch', architecture,
                            '-mmacosx-version-min='+minimum, str(source),
                            '-o', str(cls.fixtures/label)], check=True, capture_output=True)
        cls.bundles = {}
        for label in ('supported', 'newer'):
            app = cls.fixtures/(label+'-app')/'Code-Codex.app'
            binary = cls.fixtures/label
            package.create_bundle(binary, app, VERSION, binary)
            cls.bundles[label] = app

    def setUp(self):
        temp = tempfile.TemporaryDirectory(dir=self.fixtures)
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.work = self.root/'发版 stage'
        self.candidate = self.work/'Code-Codex-0.6.20-arm64.dmg'
        self.release = self.work/'dist'
        self.notary_status = 'In Progress'
        self.log_matches = True
        self.signature_flags = '0x10000(runtime)'
        self.signature_timestamp = 'Timestamp=Nov 15, 2026 at 9:00:00 AM\n'
        self.stapler_fails = False
        self.upload_times_out = False
        self.upload_partial_response = None
        self.upload_count = 0
        self.upload_sha256 = None
        self.staple_count = 0
        self.replace_candidate_on = None

    def test_read_only_check_accepts_an_actual_arm64_development_bundle(self):
        app = self.bundles['supported']
        before = (app/'Contents/Info.plist').read_bytes()
        result = cli('check', '--app', app, '--version', VERSION)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((app/'Contents/Info.plist').read_bytes(), before)
        self.assertIn('13.0', result.stdout)

    def test_check_refuses_wrong_product_version_minimum_and_architecture(self):
        app = self.root/'Code-Codex.app'
        shutil.copytree(self.bundles['supported'], app)
        info = app/'Contents/Info.plist'
        original = plistlib.loads(info.read_bytes())
        for path, version, field, value, diagnostic in [
            (app, VERSION, 'CFBundleIdentifier', 'com.openai.codex', 'CFBundleIdentifier'),
            (app, '0.7.0', None, None, 'CFBundleShortVersionString'),
            (app, '0.06.20', None, None, 'Version must'),
            (app, VERSION, 'LSMinimumSystemVersion', '14.0', 'LSMinimumSystemVersion'),
            (self.bundles['newer'], VERSION, None, None, 'minimum macOS'),
            (app, VERSION, 'binary', 'intel', 'ARM64'),
        ]:
            with self.subTest(diagnostic=diagnostic):
                plist = dict(original)
                if field == 'binary':
                    shutil.copyfile(self.fixtures/value, app/'Contents/MacOS/Code-Codex')
                elif field:
                    plist[field] = value
                info.write_bytes(plistlib.dumps(plist))
                result = cli('check', '--app', path, '--version', version)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(diagnostic, result.stderr)

    def test_prepare_rejects_existing_output_before_signing(self):
        self.work.mkdir()
        marker = self.work/'keep.txt'
        marker.write_text('owned')
        result = cli('prepare', '--app', self.bundles['supported'], '--version', VERSION,
                     '--output', self.work, '--team-id', TEAM, '--identity', IDENTITY)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('already exists', result.stderr)
        self.assertEqual(list(self.work.iterdir()), [marker])
        self.assertEqual(marker.read_text(), 'owned')

    def test_check_refuses_invalid_storage_markers_inside_valid_ad_hoc_seals(self):
        app = self.root/'Code-Codex.app'
        shutil.copytree(self.bundles['supported'], app)
        marker = app/'Contents/Resources/storage-layout.json'
        valid = b'{"schema":1,"product":"Code-Codex"}'
        for payload in (
            b'{"schema":99,"product":"Other"}',
            b'{"schema":1,"product":"Other"}',
            b'{"schema":true,"product":"Code-Codex"}',
            b'{"schema":1.0,"product":"Code-Codex"}',
            b'{"schema":1,"product":',
            b'{"schema":1,"product":"Code-Codex","extra":NaN}',
            b'{"schema":1,"product":"Code-Codex","extra":Infinity}',
            b'{"schema":1,"product":"Code-Codex","extra":1e400}',
            rb'{"schema":1,"product":"Code-Codex","extra":"\ud800"}',
            rb'{"schema":NaN,"schema":1,"product":"Code-Codex"}',
            rb'{"schema":1,"product":"\ud800","product":"Code-Codex"}',
            b'{"schema":1,"product":"Code-Codex","extra":"unused"}',
            valid.ljust(4097),
        ):
            with self.subTest(payload=payload[:60], size=len(payload)):
                marker.write_bytes(payload)
                subprocess.run(['/usr/bin/codesign', '--force', '--sign', '-', app],
                               check=True, capture_output=True)
                subprocess.run(['/usr/bin/codesign', '--verify', '--deep', '--strict', app],
                               check=True, capture_output=True)
                result = cli('check', '--app', app, '--version', VERSION)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('storage layout marker', result.stderr)

    def replace_candidate(self, process):
        payload = self.root/'different-payload'
        payload.mkdir()
        (payload/'other-build').write_text('This is a different valid disk image.')
        replacement = self.root/'different.dmg'
        process(['/usr/bin/hdiutil', 'create', '-quiet', '-fs', 'HFS+', '-format', 'UDZO',
                 '-volname', 'Different', '-srcfolder', payload, replacement],
                capture_output=True, check=True)
        replacement.replace(self.candidate)

    def test_prepare_refuses_writing_inside_an_app_even_through_symlinked_parent(self):
        outer = self.root/'Another.app'
        outer.mkdir()
        redirected = self.root/'redirected'
        redirected.symlink_to(outer, target_is_directory=True)
        result = cli('prepare', '--app', self.bundles['supported'], '--version', VERSION,
                     '--output', redirected/'new', '--team-id', TEAM, '--identity', IDENTITY)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('app bundle', result.stderr)
        self.assertEqual(list(outer.iterdir()), [])

    def Apple(self, process, args, **kwargs):
        command = [str(value) for value in args]
        if command[0] == '/usr/bin/security':
            return subprocess.CompletedProcess(args, 0,
                f'  1) {IDENTITY} "Developer ID Application: Example ({TEAM})"\n', '')
        if command[0] == '/usr/bin/codesign':
            if '--sign' in command or '-R' in command:
                if '--sign' in command:
                    self.assertIn('--timestamp', command)
                    if not command[-1].endswith('.dmg'):
                        self.assertIn('runtime', command)
                return subprocess.CompletedProcess(args, 0, '', '')
            if '--display' in command:
                digest = CORE_CDHASH if command[-1].endswith('code-codex-core') else APP_CDHASH
                return subprocess.CompletedProcess(args, 0, '',
                    f'CodeDirectory v=20500 size=123 flags={self.signature_flags} hashes=3+7\n'
                    f'CDHash={digest}\n'
                    f'{self.signature_timestamp}'
                    f'TeamIdentifier={TEAM}\n')
        if command[:2] == ['/usr/bin/xcrun', 'stapler']:
            if command[2] == 'staple':
                self.staple_count += 1
            if self.stapler_fails and command[2] == 'validate':
                return subprocess.CompletedProcess(args, 1, '', 'ticket validation failed')
            return subprocess.CompletedProcess(args, 0, '', '')
        if command[:2] == ['/usr/bin/xcrun', 'notarytool']:
            self.assertIn('--keychain-profile', command)
            operation = command[2]
            if operation == 'submit':
                self.upload_count += 1
                self.assertIn('--no-wait', command)
                if self.replace_candidate_on == 'submit':
                    self.replace_candidate(process)
                self.upload_sha256 = hashlib.sha256(Path(command[3]).read_bytes()).hexdigest()
                if self.upload_times_out:
                    raise subprocess.TimeoutExpired(args, kwargs['timeout'], output=self.upload_partial_response)
                response = {'id': JOB, 'message': 'Successfully uploaded file'}
            elif operation == 'info':
                response = {'createdDate': '2026-11-15T00:00:00Z', 'id': JOB,
                            'name': self.candidate.name, 'status': self.notary_status}
            elif operation == 'log':
                response = {'logFormatVersion': 1, 'jobId': JOB.upper(),
                            'status': self.notary_status, 'statusSummary': 'Processing complete',
                            'statusCode': 0, 'archiveFilename': self.candidate.name,
                            'uploadDate': '2026-11-15T00:00:00Z',
                            'sha256': hashlib.sha256(self.candidate.read_bytes()).hexdigest()
                            if self.log_matches else '0'*64,
                            'ticketContents': [
                                {'path': self.candidate.name+'/Code-Codex.app',
                                 'arch': 'arm64', 'cdhash': APP_CDHASH, 'digestAlgorithm': 'SHA-256'}
                            ], 'issues': None}
                if self.replace_candidate_on == 'log':
                    self.replace_candidate(process)
            else:
                self.fail(f'Unexpected Apple operation {operation}')
            return subprocess.CompletedProcess(args, 0, json.dumps(response), '')
        return process(args, **kwargs)

    def invoke(self, *args):
        import release
        process = subprocess.run
        with patch.object(release.subprocess, 'run',
                          side_effect=lambda args, **kwargs: self.Apple(process, args, **kwargs)):
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                return release.main(list(map(str, args)))

    def prepare(self):
        before = (self.bundles['supported']/'Contents/MacOS/Code-Codex').read_bytes()
        self.assertEqual(self.invoke('prepare', '--app', self.bundles['supported'], '--version',
                                     VERSION, '--output', self.work, '--team-id', TEAM,
                                     '--identity', IDENTITY), 0)
        self.assertTrue(self.candidate.is_file())
        self.assertEqual(before, (self.bundles['supported']/'Contents/MacOS/Code-Codex').read_bytes())

    def notarize(self, action, *args):
        return self.invoke(action, '--work', self.work, '--keychain-profile', 'notary-profile', *args)

    def receipt(self):
        return json.loads((self.work/'notarization.json').read_text())

    def test_verified_notarized_artifacts_still_record_blocked_distribution(self):
        self.prepare()
        self.assertEqual(self.notarize('submit'), 0)
        self.assertEqual(self.receipt()['submission_id'], JOB)
        self.assertEqual(self.receipt()['status'], 'pending')
        self.assertEqual(self.notarize('finish'), 2)
        self.assertFalse(self.release.exists())
        self.notary_status = 'Accepted'
        self.stapler_fails = True
        self.assertEqual(self.notarize('finish'), 1)
        self.assertEqual(self.receipt()['status'], 'accepted')
        self.assertFalse(self.release.exists())
        self.stapler_fails = False
        self.assertEqual(self.notarize('finish'), 0)
        published = json.loads((self.release/'release.json').read_text())
        self.assertEqual(published['status'], 'notarized-artifacts')
        self.assertEqual(published['distribution_readiness'], {
            'status': 'blocked',
            'reason': 'unconfigured-native-release-trust',
        })
        self.assertEqual(published['submission_id'], JOB)
        for suffix in ('.zip', '.dmg'):
            asset = self.release/(self.candidate.stem+suffix)
            expected = hashlib.sha256(asset.read_bytes()).hexdigest()
            self.assertEqual(published['assets'][asset.name]['sha256'], expected)
        self.assertEqual(self.notarize('finish'), 1)
        self.assertEqual(self.upload_count, 1)

    def test_rejected_job_cannot_publish(self):
        self.prepare()
        self.assertEqual(self.notarize('submit'), 0)
        self.notary_status = 'Invalid'
        self.assertEqual(self.notarize('finish'), 1)
        self.assertEqual(self.receipt()['status'], 'failed')
        self.assertFalse(self.release.exists())

    def test_ambiguous_submission_is_never_uploaded_again_and_recovery_checks_archive_hash(self):
        self.prepare()
        self.upload_times_out = True
        self.assertEqual(self.notarize('submit'), 1)
        self.assertEqual(self.receipt()['status'], 'submission-unknown')
        self.assertEqual(self.notarize('submit'), 1)
        self.assertEqual(self.upload_count, 1)
        self.notary_status = 'Accepted'
        self.log_matches = False
        self.assertEqual(self.notarize('finish', '--submission-id', JOB), 1)
        self.assertFalse(self.release.exists())
        self.assertEqual(self.receipt()['status'], 'submission-unknown')
        self.log_matches = True
        self.assertEqual(self.notarize('finish', '--submission-id', JOB), 0)
        self.assertEqual(self.receipt()['submission_id'], JOB)

    def test_changed_candidate_blocks_upload(self):
        self.prepare()
        with self.candidate.open('ab') as file:
            file.write(b'modified')
        self.assertEqual(self.notarize('submit'), 1)
        self.assertEqual(self.upload_count, 0)

    def test_submit_uploads_verified_snapshot_if_work_file_is_swapped(self):
        self.prepare()
        expected = hashlib.sha256(self.candidate.read_bytes()).hexdigest()
        self.replace_candidate_on = 'submit'
        self.assertEqual(self.notarize('submit'), 0)
        self.assertNotEqual(hashlib.sha256(self.candidate.read_bytes()).hexdigest(), expected)
        self.assertEqual(self.upload_sha256, expected)

    def test_finish_refuses_replacement_dmg_after_apple_log_checked_original(self):
        self.prepare()
        self.assertEqual(self.notarize('submit'), 0)
        self.notary_status = 'Accepted'
        self.replace_candidate_on = 'log'
        self.assertEqual(self.notarize('finish'), 1)
        self.assertEqual(self.staple_count, 0)
        self.assertFalse(self.release.exists())

    def test_timeout_preserves_an_id_already_reported_by_the_upload(self):
        self.prepare()
        self.upload_times_out = True
        self.upload_partial_response = f'{{"id":"{JOB}"'.encode()
        self.assertEqual(self.notarize('submit'), 1)
        self.assertEqual(self.receipt()['submission_id'], JOB)
        self.assertEqual(self.receipt()['status'], 'pending')
        self.assertEqual(self.notarize('finish'), 2)

    def test_recovery_remembers_the_unconfirmed_submission_while_apple_is_processing(self):
        self.prepare()
        self.upload_times_out = True
        self.assertEqual(self.notarize('submit'), 1)
        self.assertEqual(self.notarize('finish', '--submission-id', JOB), 2)
        self.assertEqual(self.receipt()['recovery_submission_id'], JOB)
        self.assertIsNone(self.receipt()['submission_id'])
        self.notary_status = 'Accepted'
        self.assertEqual(self.notarize('finish'), 0)
        self.assertEqual(self.receipt()['submission_id'], JOB)

    def test_wrong_team_identity_fails_before_any_candidate_output(self):
        self.assertEqual(self.invoke('prepare', '--app', self.bundles['supported'], '--version',
                                     VERSION, '--output', self.work, '--team-id', 'ZZZZZZZZZZ',
                                     '--identity', IDENTITY), 1)
        self.assertFalse(self.work.exists())

    def test_candidate_requires_hardened_runtime_and_a_secure_timestamp(self):
        for flags, timestamp in [('0x0(none)', self.signature_timestamp),
                                 (self.signature_flags, 'Signed Time=Nov 15, 2026\n')]:
            with self.subTest(flags=flags, timestamp=timestamp):
                self.signature_flags = flags
                self.signature_timestamp = timestamp
                self.assertEqual(self.invoke('prepare', '--app', self.bundles['supported'],
                                             '--version', VERSION, '--output', self.work,
                                             '--team-id', TEAM, '--identity', IDENTITY), 1)
                self.assertFalse(self.work.exists())


if __name__ == '__main__':
    unittest.main()
