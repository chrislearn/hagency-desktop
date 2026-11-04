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
    from PIL import Image
    frames, stable = 0, 0
    while time.monotonic() < deadline:
        try:
            with urlopen(f'http://127.0.0.1:{port}/g?w=0&wait=1', timeout=10) as response:
                frame = json.load(response)
            shutil.copy2(frame['png'], out/'window.png')
            frames += 1
            pixels = list(Image.open(frame['png']).convert('RGB').resize((100, 100)).getdata())
            red = sum(r > 200 and g < 120 and b < 150 for r,g,b in pixels)
            blue = sum(b > r*1.4 and b > g and b > 40 for r,g,b in pixels)
            stable = stable+1 if red > 10 and blue > 10 else 0
            if stable == 3: break
        except OSError:
            pass
        time.sleep(.5)
    from PIL import ImageGrab
    ImageGrab.grab(all_screens=True).save(out/'desktop.png')
    (out/'report.json').write_text(json.dumps(dict(passed=stable==3, frames=frames,
        stableFrames=stable, inputInjected=False), indent=2)+'\n')
    print('Minimal renderer frames:', frames)
    assert stable == 3, 'Idle first window did not render both colored surfaces'
finally:
    process.terminate()
    try: process.wait(timeout=10)
    except subprocess.TimeoutExpired: process.kill(); process.wait(timeout=5)
