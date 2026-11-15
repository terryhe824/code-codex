#!/usr/bin/env python3
"""Package a locally built ARM64 core; never modifies the official Codex app."""
import argparse
import ctypes
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import struct
import subprocess
import tempfile

REPO = Path(__file__).resolve().parents[2]
BUNDLE_ID = 'io.github.rice-dog.code-codex.macos'
PRODUCT = 'Code-Codex'
MIN_MACOS = '13.0'
STORAGE_LAYOUT_NAME = 'storage-layout.json'
STORAGE_LAYOUT_CONTENT = (json.dumps({'schema': 1, 'product': PRODUCT})+'\n').encode('utf-8')
RESOURCE_FILES = ('LICENSE', 'THIRD_PARTY_NOTICES_EN.md', 'THIRD_PARTY_NOTICES_ZH_CN.md',
                  'THIRD_PARTY_LICENSES.txt')


def rename_exclusive(source, destination):
    # Darwin RENAME_EXCL makes the absence check atomic with publication.
    libc = ctypes.CDLL('/usr/lib/libSystem.B.dylib', use_errno=True)
    rename = libc.renamex_np
    rename.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint]
    rename.restype = ctypes.c_int
    if rename(os.fsencode(source), os.fsencode(destination), 0x00000004) != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error), str(destination))


def create_bundle(binary, destination, version, launcher=None):
    binary, destination = Path(binary), Path(destination)
    if not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+', version):
        raise ValueError('version must have three numeric components')
    if os.path.lexists(destination):
        raise FileExistsError(destination)
    if destination.suffix != '.app':
        raise ValueError('destination must be an .app bundle')
    if binary.is_symlink() or not binary.is_file():
        raise ValueError('core must be a regular ARM64 Mach-O file')
    with binary.open('rb') as source:
        header = source.read(8)
    if len(header) != 8 or struct.unpack('<II', header) != (0xfeedfacf, 0x0100000c):
        raise ValueError('core must be a native ARM64 Mach-O executable')
    launcher = Path(launcher) if launcher is not None else binary.with_name('code-codex-launcher')
    if launcher.is_symlink() or not launcher.is_file() or launcher.read_bytes()[:8] != header:
        raise ValueError('GUI launcher must be a regular ARM64 Mach-O file')
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.code-codex-package-', dir=destination.parent) as stage:
        bundle = Path(stage)/destination.name
        executable = bundle/'Contents/MacOS/Code-Codex'
        resources = bundle/'Contents/Resources'
        executable.parent.mkdir(parents=True)
        resources.mkdir()
        shutil.copyfile(launcher, executable)
        executable.chmod(0o755)
        core = executable.with_name('code-codex-core')
        shutil.copyfile(binary, core)
        core.chmod(0o755)
        subprocess.run(['/usr/bin/codesign', '--force', '--sign', '-', str(core)], check=True, capture_output=True)
        for name in RESOURCE_FILES:
            shutil.copyfile(REPO/name, resources/name)
        (resources/STORAGE_LAYOUT_NAME).write_bytes(STORAGE_LAYOUT_CONTENT)
        info = {'CFBundleIdentifier': BUNDLE_ID, 'CFBundleName': 'Code-Codex',
                'CFBundleDisplayName': 'Code-Codex', 'CFBundleExecutable': 'Code-Codex',
                'CFBundlePackageType': 'APPL', 'CFBundleShortVersionString': version,
                'CFBundleVersion': version, 'LSMinimumSystemVersion': MIN_MACOS,
                'NSHighResolutionCapable': True}
        (bundle/'Contents/Info.plist').write_bytes(plistlib.dumps(info))
        # Local development signature only. Public release needs Developer ID + notarization.
        subprocess.run(['/usr/bin/codesign', '--force', '--sign', '-', str(bundle)], check=True, capture_output=True)
        subprocess.run(['/usr/bin/codesign', '--verify', '--strict', str(bundle)], check=True, capture_output=True)
        rename_exclusive(bundle, destination)
    return destination


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=REPO/'target/release/code-codex')
    parser.add_argument('--launcher', type=Path, help='GUI launcher (defaults to sibling code-codex-launcher)')
    parser.add_argument('--output', type=Path, required=True, help='new output directory; existing output is refused')
    parser.add_argument('--version', required=True)
    args = parser.parse_args()
    if os.path.lexists(args.output):
        parser.error('output already exists; select a new output directory')
    args.output.mkdir(parents=True)
    bundle = create_bundle(args.binary, args.output/'Code-Codex.app', args.version, args.launcher)
    archive = args.output/('Code-Codex-'+args.version+'-arm64.zip')
    subprocess.run(['/usr/bin/ditto', '-c', '-k', '--sequesterRsrc', '--keepParent', str(bundle), str(archive)], check=True)
    with tempfile.TemporaryDirectory(prefix='.dmg-stage-', dir=args.output) as temp:
        shutil.copytree(bundle, Path(temp)/bundle.name)
        subprocess.run(['/usr/bin/hdiutil', 'create', '-quiet', '-fs', 'HFS+', '-format', 'UDZO',
                        '-volname', 'Code-Codex', '-srcfolder', temp,
                        str(args.output/('Code-Codex-'+args.version+'-arm64.dmg'))], check=True)
    print('Created local development app, ZIP and DMG (ad-hoc signed; not notarized): '+str(args.output))


if __name__ == '__main__':
    main()
