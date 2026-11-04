#!/usr/bin/env python3
"""Build a local macOS controller bundle and its independent daemon helper.

Does not install a LaunchAgent, alter login state, launch apps or deploy files.
Use --release for an optimized build. Run from any working directory.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
IDENTITY = 'io.hagency.owner-controller'


def run(args, cwd=ROOT, env=None, capture=False):
    return subprocess.run(args, cwd=cwd, env=env, check=True,
                          text=True, stdout=subprocess.PIPE if capture else None).stdout


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            value.update(chunk)
    return value.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--release', action='store_true')
    parser.add_argument('--client', type=Path, default=ROOT.parent / 'hagency-client')
    parser.add_argument('--output', type=Path, default=ROOT / '.run/owner-daemon/dist')
    parser.add_argument('--dmg', action='store_true', help='Also create the distributable macOS disk image')
    args = parser.parse_args()
    if sys.platform != 'darwin':
        parser.error('This bundle is for macOS.')
    client, output = args.client.resolve(), args.output.resolve()
    if not (client / 'native/hagency/Cargo.toml').is_file():
        parser.error('--client must point to the Hagency Client repository.')
    output.mkdir(parents=True, exist_ok=True)
    destination = output / 'Hagency Hosting.app'
    if destination.exists():
        parser.error(f'{destination} already exists; choose a new --output to preserve it.')
    # The helper must come from the published Client revision used by the UI,
    # not whichever dirty sibling checkout happens to be present on this Mac.
    pin = re.search(r'^hagency = \{ git = "https://github.com/hagency-org/hagency-client.git", rev = "([0-9a-f]{40})"',
                    (ROOT / 'Cargo.toml').read_text(), re.M)
    if not pin or run(['git', 'rev-parse', 'HEAD'], cwd=client, capture=True).strip() != pin[1]:
        parser.error('--client must be checked out at Desktop’s pinned Client revision.')
    if run(['git', 'status', '--porcelain', '--untracked-files=all'], cwd=client, capture=True).strip():
        parser.error('--client must be clean; uncommitted helper code cannot be packaged.')
    # The accessibility patch intentionally changes Makepad package identities.
    # Resolve its special lockfiles in a copy, leaving standard release/CI
    # lockfiles untouched and reproducible after packaging.
    source = output / 'native-source'
    if source.exists():
        parser.error(f'{source} already exists; choose a new --output.')
    for relative in ['apps/owner-controller', 'crates/owner-dashboard']:
        shutil.copytree(ROOT / relative, source / relative,
                        ignore=shutil.ignore_patterns('target', '__pycache__'))
    fonts = Path('crates/article-makepad/src/apple_fonts.rs')
    (source / fonts).parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(ROOT / fonts, source / fonts)
    controller = source / 'apps/owner-controller/Cargo.toml'
    env = os.environ.copy()
    env['MAKEPAD'] = 'apple_bundle'
    env['MAKEPAD_PACKAGE_DIR'] = '.'
    env['RUST_MIN_STACK'] = env.get('RUST_MIN_STACK', '16777216')
    # Packaging must never overwrite the instrumentation build while a review
    # is running. The helper keeps its existing Client target directory.
    controller_env = dict(env, CARGO_TARGET_DIR=str(ROOT / '.run/owner-daemon/targets/controller'))
    native_config = output / 'native-accessibility.cargo.toml'
    run([sys.executable, str(ROOT / 'tools/prepare-owner-accessibility.py'),
         '--output-config', str(native_config)])
    profile = ['--release'] if args.release else []
    config = f'env.MAKEPAD_BUNDLE_IDENTIFIER.value="{IDENTITY}"'
    # Remap the pinned framework source to the reviewed local patch before
    # locked compilation. A clean checkout's ordinary lockfile contains Git
    # package identities, while this native build intentionally uses paths.
    metadata = json.loads(run(['cargo', 'metadata', '--format-version', '1',
                              '--manifest-path', str(controller), '--features', 'native-accessibility',
                              '--config', str(native_config)], env=controller_env, capture=True))
    # Cargo's per-target patch-unused set can differ from metadata's all-target
    # graph. Let this first pinned-path build finish remapping the lockfile.
    run(['cargo', 'build', '--manifest-path', str(controller),
         '--bin', 'hagency-owner-controller', '--features', 'native-accessibility',
         '--config', config, '--config', str(native_config), *profile], env=controller_env)
    run(['cargo', 'build', '--locked', '-p', 'hagency', '--bin', 'hagency',
         '--features', 'shared-budget-execution', *profile], cwd=client, env=env)
    helper_meta = json.loads(run(['cargo', 'metadata', '--locked', '--no-deps',
                                 '--format-version', '1'], cwd=client, env=env, capture=True))
    build_profile = 'release' if args.release else 'debug'
    controller_bin = Path(metadata['target_directory']) / build_profile / 'hagency-owner-controller'
    helper_bin = Path(helper_meta['target_directory']) / build_profile / 'hagency'
    with tempfile.TemporaryDirectory(prefix='owner-bundle-', dir=output) as tmp:
        bundle = Path(tmp) / destination.name
        contents = bundle / 'Contents'
        for name in ['MacOS', 'Helpers', 'Resources', 'Resources/Licenses']:
            (contents / name).mkdir(parents=True, exist_ok=True)
        shutil.copy2(controller_bin, contents / 'MacOS/hagency-owner-controller')
        shutil.copy2(helper_bin, contents / 'Helpers/hagency')
        shutil.copy2(ROOT / 'resources/icon.icns', contents / 'Resources/icon.icns')
        resources = []
        for package in metadata['packages']:
            source = Path(package['manifest_path']).parent
            package_name = package['name'].replace('-', '_')
            if (source / 'resources').is_dir():
                target = contents / 'Resources' / package_name / 'resources'
                shutil.copytree(source / 'resources', target)
                resources.append(package['name'])
            for license_file in source.glob('LICENSE*'):
                if license_file.is_file():
                    shutil.copy2(license_file, contents / 'Resources/Licenses' /
                                 f'{package_name}-{package["version"]}-{license_file.name}')
        release_version = re.search(r'^version\s*=\s*"([^"]+)"', (ROOT / 'Cargo.toml').read_text(), re.M)[1]
        info = {
            'CFBundleIdentifier': IDENTITY,
            'CFBundleName': 'Hagency Hosting',
            'CFBundleDisplayName': 'Hagency Hosting',
            'CFBundleExecutable': 'hagency-owner-controller',
            'CFBundlePackageType': 'APPL',
            'CFBundleShortVersionString': release_version.split('-')[0],
            'CFBundleVersion': os.environ.get('GITHUB_RUN_NUMBER', '1'),
            'HagencyReleaseVersion': release_version,
            'CFBundleIconFile': 'icon.icns',
            'NSHighResolutionCapable': True,
            'LSMinimumSystemVersion': '11.0',
            'LSUIElement': False,
        }
        with (contents / 'Info.plist').open('wb') as stream:
            plistlib.dump(info, stream, sort_keys=True)
        for path in [contents / 'Helpers/hagency', contents / 'MacOS/hagency-owner-controller', bundle]:
            run(['/usr/bin/codesign', '--force', '--sign', '-', str(path)])
        run(['/usr/bin/codesign', '--verify', '--deep', '--strict', str(bundle)])
        report = {
            'bundleId': IDENTITY, 'profile': build_profile, 'signing': 'ad-hoc',
            'nativeAccessibility': True,
            'makepadPatchSha256': digest(ROOT / 'packaging/owner-controller/makepad-accessibility.patch'),
            'controllerSha256': digest(contents / 'MacOS/hagency-owner-controller'),
            'daemonSha256': digest(contents / 'Helpers/hagency'),
            'bundledResourceCrates': sorted(resources),
            'desktopCommit': run(['git', 'rev-parse', 'HEAD'], capture=True).strip(),
            'clientCommit': run(['git', 'rev-parse', 'HEAD'], cwd=client, capture=True).strip(),
            'desktopTrackedDiffSha256': hashlib.sha256(run(['git', 'diff', 'HEAD'], capture=True).encode()).hexdigest(),
            'clientTrackedDiffSha256': hashlib.sha256(run(['git', 'diff', 'HEAD'], cwd=client, capture=True).encode()).hexdigest(),
            'version': release_version,
            'languages': ['en', 'zh-Hans'],
            'languageSelection': 'persistent, with system default',
            'installed': False, 'launchAgentInstalled': False,
        }
        bundle.rename(destination)
        (output / 'build-report.json').write_text(json.dumps(report, indent=2) + '\n')
    if args.dmg:
        with tempfile.TemporaryDirectory(prefix='hosting-dmg-', dir=output) as tmp:
            stage = Path(tmp)
            run(['ditto', str(destination), str(stage / destination.name)])
            (stage / 'Applications').symlink_to('/Applications')
            dmg = output / 'Hagency Hosting.dmg'
            run(['hdiutil', 'create', '-volname', 'Hagency Hosting', '-srcfolder', str(stage),
                 '-format', 'UDZO', '-imagekey', 'zlib-level=9', str(dmg)])
        run(['bash', str(ROOT / 'packaging/fix-dmg-applications-icon.sh'), str(dmg),
             str(ROOT / 'packaging/macos/dmg-background.png'), destination.name])
        run(['hdiutil', 'verify', str(dmg)])
    print(destination)


if __name__ == '__main__':
    main()
