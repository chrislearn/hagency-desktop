#!/usr/bin/env python3
"""Measure actual rendered glyph growth from the host stylesheet scale.

Both runs use the same `miniapp_dock` executable, dashboard scale 1, and an
isolated hidden native fixture with an offline profile, and measure the Usage
total. Only the host typography multiplier changes; no system setting changes.
"""
import argparse, hashlib, importlib.util, json, os, re, subprocess, time
from pathlib import Path
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('owner_ax', ROOT/'tools/owner-dashboard-ax.py')
ax = importlib.util.module_from_spec(spec); spec.loader.exec_module(ax)


def measure(binary, out, scale):
    out.mkdir(parents=True, exist_ok=True)
    logfile = out/'fixture.log'
    env = dict(os.environ, MAKEPAD_HIDE_WINDOWS='1', HAGENCY_OWNER_REVIEW_HOST_SCALE=str(scale))
    env.pop('MAKEPAD_FOCUS', None)
    command = [str(binary), '--remote']
    profile=out/'profile';profile.mkdir(exist_ok=True);(profile/'.article-review-profile').touch();(profile/'ui-language.json').write_text('"en"')
    env.update(HAGENCY_DESKTOP_DATA_DIR=str(profile),HAGENCY_DOCK_OFFLINE_ACCOUNT='1',HAGENCY_OWNER_REVIEW_WIDTH='420',HAGENCY_OWNER_REVIEW_SCALE='1')
    process = journey = None
    try:
        with logfile.open('w') as log:
            process = subprocess.Popen(command, cwd=ROOT,
                env=env, stdout=log, stderr=subprocess.STDOUT)
            deadline = time.monotonic()+30
            while time.monotonic() < deadline:
                found = re.search(r'listening on 127.0.0.1:(\d+) pid=(\d+)', logfile.read_text())
                if found: break
                assert process.poll() is None, 'fixture exited'
                time.sleep(.1)
            assert found and int(found[2]) == process.pid, 'owned fixture endpoint missing'
            journey = ax.Journey('http://127.0.0.1:'+found[1], out)
            deadline = time.monotonic()+20
            while not journey.get('/s').get('w'):
                assert time.monotonic() < deadline, 'window missing'
                time.sleep(.1)
            journey.scenario('running');journey.get('/g')
            visible=journey.get('/snap')['s']
            if any(r.get('i')=='od_destination' and r['r'][2]>0 for r in visible):
                journey.press('Dashboard page','open-page')
            journey.press('Usage','usage')
            ident='total'
            journey.grab('populated')
            bounds = next(n['r'] for n in journey.get('/snap', q=ident)['s'] if n.get('i') == ident)
            dpi = journey.get('/s')['w'][0]['dpi']
            x,y,w,h = bounds
            # First characters of the Usage total only.
            image = Image.open(out/'populated.png').convert('RGB')
            sample = image.crop((int((x+8)*dpi), int(y*dpi), int((x+8+32*scale)*dpi), int((y+h)*dpi)))
            rows = [row for row in range(sample.height) if any(max(sample.getpixel((col,row))) < 120 for col in range(sample.width))]
            assert rows, 'populated glyphs missing'
            return {'hostScale':scale, 'dashboardScale':1, 'widget':ident, 'bounds':bounds, 'dpi':dpi,
                    'inkHeightPhysical':max(rows)-min(rows)+1}
    finally:
        if process:
            if journey and process.poll() is None:
                try: journey.get('/quit')
                except Exception: pass
            try: process.wait(timeout=10)
            except subprocess.TimeoutExpired: process.terminate(); process.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args(); binary=args.binary.resolve(); out=args.output.resolve(); out.mkdir(parents=True, exist_ok=True)
    report = {'binary':str(binary), 'binarySha256':hashlib.sha256(binary.read_bytes()).hexdigest(), 'passed':False}
    try:
        normal=measure(binary, out/'normal', 1)
        enlarged=measure(binary, out/'host-150', 1.5)
        ratio=enlarged['inkHeightPhysical']/normal['inkHeightPhysical']
        report.update(normal=normal, enlarged=enlarged, glyphGrowth=ratio)
        assert 1.35 <= ratio <= 1.65, 'host150 must enlarge actual populated glyphs by approximately 1.5'
        assert enlarged['bounds'][3] > normal['bounds'][3], 'host typography must also enlarge the widget bounds'
        report['passed']=True
    except Exception as error:
        report['error']=str(error)
        raise
    finally:
        (out/'report.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report))


if __name__ == '__main__': main()
