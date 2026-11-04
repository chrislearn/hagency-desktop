#!/usr/bin/env python3
"""Install/relocate a packaged Desktop and render sign-in with fresh app data."""
import argparse
from contextlib import contextmanager, ExitStack
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time
from urllib.request import urlopen
from urllib.error import HTTPError
from release_image_check import image_readiness


@contextmanager
def software_renderer(binary):
    """Select WARP for this temporary executable on GPU-less Windows CI hosts.

    This is the per-application DirectX Control Panel override; it does not
    change the distributed executable or the machine's global GPU selection.
    """
    import winreg
    parent = r'Software\Microsoft\Direct3D'
    for index in range(10000):
        path = parent + r'\Application' + str(index)
        try:
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER, path):
                pass
        except FileNotFoundError:
            break
    else:
        raise RuntimeError('No free DirectX application override slot')
    try:
        with winreg.CreateKey(winreg.HKEY_CURRENT_USER, path) as key:
            winreg.SetValueEx(key, 'Name', 0, winreg.REG_SZ, str(binary))
            winreg.SetValueEx(key, 'RequireSDKLayers', 0, winreg.REG_DWORD, 0)
            winreg.SetValueEx(key, 'D3DBehaviors', 0, winreg.REG_SZ,
                'RequireSDKLayers=0;FeatureLevelLimit=0;ForceWARP=1;DisableFLUpgrade=0')
        yield
    finally:
        winreg.DeleteKey(winreg.HKEY_CURRENT_USER, path)

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--package', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--source-root', type=Path, default=Path(__file__).resolve().parents[1],
                    help='Source checkout that produced the installer (for its kernel pin)')
parser.add_argument('--software-rendering', action='store_true',
                    help='Use the per-application WARP override on an isolated Windows test host')
args = parser.parse_args()
if args.software_rendering and sys.platform != 'win32':
    parser.error('--software-rendering is only available on Windows')
package, output = args.package.resolve(), args.output.resolve()
output.mkdir(parents=True, exist_ok=True)

with tempfile.TemporaryDirectory(prefix='hagency-release-smoke-') as temporary, ExitStack() as cleanup:
    temporary = Path(temporary)
    if sys.platform == 'darwin':
        if package.suffix.lower() == '.dmg':
            mount = temporary/'image'
            mount.mkdir()
            # cargo-packager removes its intermediate .app after creating a
            # DMG. Exercise the distributable itself, including its contents.
            # The EULA is our own bundled application license.
            subprocess.run(['hdiutil', 'attach', '-readonly', '-nobrowse',
                            '-mountpoint', str(mount), str(package)],
                           input='Y\n', text=True, check=True, timeout=120)
            try:
                candidates = list(mount.glob('*.app'))
                assert len(candidates) == 1, 'Disk image must contain one Desktop app'
                app = temporary/candidates[0].name
                shutil.copytree(candidates[0], app)
            finally:
                subprocess.run(['hdiutil', 'detach', str(mount)], check=True, timeout=30)
        else:
            app = temporary/package.name
            shutil.copytree(package, app)
        binary = app/'Contents/MacOS/rinx'
        kernel = app/'Contents/MacOS/octos'
        subprocess.run(['codesign','--verify','--deep','--strict',str(app)], check=True)
    elif sys.platform == 'win32':
        install = temporary/'installed'
        subprocess.run([str(package), '/S', '/D='+str(install)], timeout=120, check=True)
        candidates = list(install.rglob('rinx.exe'))
        assert len(candidates) == 1, 'Installer did not install one Desktop executable'
        binary = candidates[0]
        kernel = binary.with_name('octos.exe')
        version_info = json.loads(subprocess.check_output(
            ['powershell', '-NoProfile', '-NonInteractive', '-Command',
             '$v=(Get-Item -LiteralPath $env:HAGENCY_SMOKE_BINARY).VersionInfo; '
             '@{product=$v.ProductName;company=$v.CompanyName}|ConvertTo-Json -Compress'],
            env=dict(os.environ, HAGENCY_SMOKE_BINARY=str(binary)), text=True, timeout=20))
        assert version_info == {'product': 'Hagency Desktop', 'company': 'Hagency'}, version_info
        if args.software_rendering:
            cleanup.enter_context(software_renderer(binary))
    else:
        raise SystemExit('This smoke check covers macOS and Windows installers')
    kernel_version = subprocess.check_output([str(kernel),'--version'], text=True, timeout=20).strip()
    pin = json.loads((args.source_root/'packaging/octos.lock.json').read_text())['revision']
    revision = re.search(r'\(([0-9a-f]{7,40}) ', kernel_version)
    assert revision and pin.startswith(revision[1]), kernel_version
    env = dict(os.environ, HAGENCY_DESKTOP_DATA_DIR=str(temporary/'profile'),
               MAKEPAD_HIDE_WINDOWS='1', MAKEPAD_REMOTE='127.0.0.1:0')
    log = output/'startup.log'
    with log.open('w') as stream:
        process = subprocess.Popen([str(binary)], cwd=temporary, env=env, stdout=stream, stderr=stream)
    try:
        deadline = time.monotonic()+90
        port = None
        while time.monotonic() < deadline:
            assert process.poll() is None, log.read_text(errors='replace')[-3000:]
            match = re.search(r'\[makepad-remote\] listening on 127\.0\.0\.1:(\d+)', log.read_text(errors='replace'))
            if match: port = int(match[1]); break
            time.sleep(.2)
        assert port, 'Packaged app did not become ready'
        def request(path):
            with urlopen(f'http://127.0.0.1:{port}/'+path, timeout=15) as response:
                return json.load(response)
        frames, ready_frames = 0, 0
        while time.monotonic() < deadline and ready_frames < 3:
            assert process.poll() is None, log.read_text(errors='replace')[-3000:]
            try:
                snapshot = request('snap?w=0&wait=1')
                image = request('g?w=0&wait=1')
                frames += 1
                texts = [node.get('t','') for node in snapshot.get('s',[])
                         if node.get('r') and node['r'][2]>0 and node['r'][3]>0]
                sign_in = any('Sign in to Hagency' in text or '登录 Hagency' in text for text in texts)
                pixels = image_readiness(image['png'])
                # Preserve the last capture even on failure, so an empty GPU
                # frame cannot be hidden by a passing semantic snapshot.
                shutil.copy2(image['png'], output/'sign-in.png')
                ready_frames = ready_frames + 1 if sign_in and pixels['nonBlank'] else 0
            except HTTPError as error:
                if error.code != 404: raise
            time.sleep(.4)
        assert ready_frames == 3, 'Packaged app did not render sign-in pixels before the deadline'
        errors = [line for line in log.read_text(errors='replace').splitlines()
                  if re.search(r'panicked at|Bundle[d]? file open failed|failed to load.*(font|resource)',line,re.I)]
        assert not errors, errors
        (output/'report.json').write_text(json.dumps(dict(passed=True, platform=sys.platform,
            package=str(package), isolatedProfile=True, kernelVersion=kernel_version,
            signInRendered=True, nonBlankScreenshot=pixels, framesChecked=frames,
            softwareRendering=args.software_rendering, resourceErrors=errors), indent=2)+'\n')
        print('Packaged Desktop started and rendered sign-in with bundled resources')
    finally:
        if process.poll() is None:
            process.terminate()
            try: process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill(); process.wait(timeout=5)
