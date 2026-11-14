import importlib.util
import pathlib
import plistlib
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('mac_package', pathlib.Path(__file__).with_name('package.py'))
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)

class PackageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='code-codex-package-', dir='/private/tmp')
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name)
        self.binary = self.root/'core with spaces'
        source = self.root/'fixture.c'
        source.write_text('int main(void) { return 0; }\n')
        subprocess.run(['/usr/bin/clang', str(source), '-o', str(self.binary)], check=True, capture_output=True)

    def test_bundle_metadata_and_core_work_in_unicode_destination(self):
        destination = self.root/'中文 destination'/'Code-Codex.app'
        package.create_bundle(self.binary, destination, '0.6.20', self.binary)
        info = plistlib.loads((destination/'Contents/Info.plist').read_bytes())
        self.assertEqual(info['CFBundleIdentifier'], 'io.github.rice-dog.code-codex.macos')
        self.assertEqual(info['CFBundleExecutable'], 'Code-Codex')
        self.assertEqual(info['CFBundleShortVersionString'], '0.6.20', self.binary)
        self.assertEqual(subprocess.run([str(destination/'Contents/MacOS/Code-Codex')]).returncode, 0)
        subprocess.run(['/usr/bin/codesign', '--verify', '--strict', str(destination)], check=True, capture_output=True)
        self.assertTrue((destination/'Contents/Resources/LICENSE').is_file())
        self.assertEqual(subprocess.run([str(destination/'Contents/MacOS/code-codex-core')]).returncode, 0)
        self.assertNotEqual((destination/'Contents/MacOS/Code-Codex').stat().st_ino, (destination/'Contents/MacOS/code-codex-core').stat().st_ino)
        self.assertEqual(self.binary.read_bytes()[:4], (destination/'Contents/MacOS/Code-Codex').read_bytes()[:4])

    def test_existing_destination_is_not_overwritten(self):
        destination = self.root/'existing.app'
        destination.mkdir(); (destination/'keep').write_text('owned')
        with self.assertRaises(FileExistsError):
            package.create_bundle(self.binary, destination, '0.6.20', self.binary)
        self.assertEqual((destination/'keep').read_text(), 'owned')

    def test_wrong_binary_and_version_are_rejected_before_writes(self):
        self.binary.write_text('not a Mach-O executable')
        destination = self.root/'bad.app'
        with self.assertRaises(ValueError): package.create_bundle(self.binary, destination, '0.6.20', self.binary)
        self.assertFalse(destination.exists())
        with self.assertRaises(ValueError): package.create_bundle(self.binary, destination, '../bad')
        self.assertFalse(destination.exists())

if __name__ == '__main__': unittest.main()
