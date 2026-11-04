"""Capture a minimal native renderer without any Hagency application code."""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time
from urllib.request import urlopen

out = Path('.run/verified-installer/probe')
out.mkdir(parents=True, exist_ok=True)
env = dict(os.environ, MAKEPAD_REMOTE='127.0.0.1:0')
env.pop('MAKEPAD_HIDE_WINDOWS', None)
with (out/'startup.log').open('w') as stream:
    process = subprocess.Popen([sys.argv[1]], env=env, stdout=stream, stderr=stream)
try:
    deadline = time.monotonic()+30
    port = None
    while time.monotonic() < deadline:
        assert process.poll() is None
        match = re.search(r'listening on 127\.0\.0\.1:(\d+)', (out/'startup.log').read_text(errors='replace'))
        if match:
            port = int(match[1]); break
        time.sleep(.2)
    assert port
    frames = 0
    while time.monotonic() < deadline:
        try:
            with urlopen(f'http://127.0.0.1:{port}/g?w=0&wait=1', timeout=10) as response:
                frame = json.load(response)
            shutil.copy2(frame['png'], out/'window.png')
            frames += 1
        except OSError:
            pass
        time.sleep(.5)
    from PIL import ImageGrab
    ImageGrab.grab(all_screens=True).save(out/'desktop.png')
    print('Minimal renderer frames:', frames)
finally:
    process.terminate()
    try: process.wait(timeout=10)
    except subprocess.TimeoutExpired: process.kill(); process.wait(timeout=5)
