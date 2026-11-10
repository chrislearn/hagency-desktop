#!/usr/bin/env python3
"""Check a persistent notification in a hidden, isolated native window.

Build the theme fixture with --profile fast --features palpo-instrument. Hidden
macOS windows can be throttled, so counts are regression evidence rather than
a representative desktop frame-rate benchmark.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import time
import uuid

from native_multi_account import App


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, default=Path('target/fast/examples/theme_mvp'))
    parser.add_argument('--assert-idle', action='store_true')
    args = parser.parse_args()
    root = Path('target/popup-performance-validation') / uuid.uuid4().hex
    profile = root / 'profile'
    profile.mkdir(parents=True, mode=0o700)
    with socket.socket() as probe:
        probe.bind(('127.0.0.1', 0))
        port = probe.getsockname()[1]
    app = App(root, port=port, auto_login=False)
    app.output.mkdir(parents=True)
    app.log = (app.output / 'native.log').open('w')
    report_path = root / 'capture.json'
    env = dict(os.environ, HAGENCY_DESKTOP_DATA_DIR=str(profile.resolve()),
               RINX_DATA_DIR=str(profile.resolve()), MAKEPAD_REMOTE=str(port),
               MAKEPAD_HIDE_WINDOWS='1', MAKEPAD_NO_FOCUS='1',
               HAGENCY_PERF_OUTPUT=str(report_path.resolve()))
    env.pop('MAKEPAD_FOCUS', None)
    app.process = subprocess.Popen([str(args.binary.resolve())], env=env, stdin=subprocess.DEVNULL,
                                   stdout=app.log, stderr=subprocess.STDOUT)
    try:
        for _ in range(120):
            if app.process.poll() is not None:
                raise RuntimeError(f'Theme fixture exited; see {app.output}')
            try:
                if app.request('/s')['pid'] == app.process.pid:
                    break
            except OSError:
                pass
            time.sleep(.1)
        app.wait_text('One theme, three native surfaces')
        app.request('/event', data='theme:popup-static')
        app.wait_text('Persistent theme notification', pixels=True)
        time.sleep(1)
        app.request('/event', data='perf:start')
        time.sleep(5)
        app.request('/event', data='perf:stop')
        for _ in range(40):
            if report_path.exists():
                break
            time.sleep(.05)
        report = json.loads(report_path.read_text())
        app.wait_text('Persistent theme notification', pixels=True)
        report.update({'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(),
                       'sample_seconds': 5, 'headless': True})
        report_path.write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps({'report': str(report_path), 'frames_painted': report['frames_painted']}), flush=True)
        if args.assert_idle:
            assert report['frames_painted'] < 30, report
    finally:
        app.stop()


if __name__ == '__main__':
    main()
