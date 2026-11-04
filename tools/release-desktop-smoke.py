#!/usr/bin/env python3
"""Install/relocate a packaged Desktop and render sign-in with fresh app data."""
import argparse
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

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--package', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
package, output = args.package.resolve(), args.output.resolve()
output.mkdir(parents=True, exist_ok=True)

with tempfile.TemporaryDirectory(prefix='hagency-release-smoke-') as temporary:
    temporary = Path(temporary)
    if sys.platform == 'darwin':
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
    else:
        raise SystemExit('This smoke check covers macOS and Windows installers')
    kernel_version = subprocess.check_output([str(kernel),'--version'], text=True, timeout=20).strip()
    pin = json.loads((Path(__file__).resolve().parents[1]/'packaging/octos.lock.json').read_text())['revision']
    revision = re.search(r'\(([0-9a-f]{7,40}) ', kernel_version)
    assert revision and pin.startswith(revision[1]), kernel_version
    env = dict(os.environ, HAGENCY_DESKTOP_DATA_DIR=str(temporary/'profile'),
               MAKEPAD_HIDE_WINDOWS='1', MAKEPAD_REMOTE='127.0.0.1:0')
    log = output/'startup.log'
    with log.open('w') as stream:
        process = subprocess.Popen([str(binary)], cwd=temporary, env=env, stdout=stream, stderr=stream)
    try:
        deadline = time.monotonic()+60
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
        for _ in range(5):
            snapshot = request('snap?w=0&wait=1')
            image = request('g?w=0&wait=1')
            time.sleep(.4)
        texts = [node.get('t','') for node in snapshot.get('s',[]) if node.get('r') and node['r'][2]>0 and node['r'][3]>0]
        assert any('Sign in to Hagency' in text or '登录 Hagency' in text for text in texts), texts
        shutil.copy2(image['png'], output/'sign-in.png')
        errors = [line for line in log.read_text(errors='replace').splitlines()
                  if re.search(r'panicked at|Bundle[d]? file open failed|failed to load.*(font|resource)',line,re.I)]
        assert not errors, errors
        (output/'report.json').write_text(json.dumps(dict(passed=True, platform=sys.platform,
            package=str(package), isolatedProfile=True, kernelVersion=kernel_version,
            signInRendered=True, resourceErrors=errors), indent=2)+'\n')
        print('Packaged Desktop started and rendered sign-in with bundled resources')
    finally:
        if process.poll() is None:
            process.terminate()
            try: process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill(); process.wait(timeout=5)
