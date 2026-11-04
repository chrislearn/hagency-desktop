#!/usr/bin/env python3
"""Exercise Agent Mermaid replies and native copy offline, without OS clipboard IO.

cargo build --locked --example chat_mermaid
python3 tools/wechat-ux/live/native_chat_mermaid.py --visible --output .run/chat-mermaid
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import time

from native_probe import NativeApp


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/debug/examples/chat_mermaid'))
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--visible', action='store_true', help='Show without focus for complete visual captures')
    args = parser.parse_args()
    root = args.output.resolve()
    root.mkdir(parents=True, exist_ok=True)
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    app = NativeApp(root, port, auto_login=False)
    app.output.mkdir(parents=True)
    app.log = (app.output / 'native.log').open('w')
    env = dict(os.environ, MAKEPAD_REMOTE=str(port), MAKEPAD_NO_FOCUS='1',
               HAGENCY_DESKTOP_DATA_DIR=str(root / 'profile'))
    if args.visible:
        env.pop('MAKEPAD_HIDE_WINDOWS', None)
    else:
        env['MAKEPAD_HIDE_WINDOWS'] = '1'
    app.process = subprocess.Popen(
        [str(args.binary.resolve())], stdin=subprocess.DEVNULL,
        stdout=app.log, stderr=subprocess.STDOUT,
        env=env,
    )
    report = {'passed': False, 'checks': [], 'window_visible': args.visible,
              'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest()}
    source = Path('tests/fixtures/grapheneos.mmd').read_text()

    def inspect(name, expected_source, diagrams):
        # Capture the diagram before selecting it to exercise native copy.
        app.capture(name)
        body = next(w for w in app.snap() if w['i'] == 'html' and w['ty'] == 'Html')
        x, y, _, _ = body['r']
        app.click(x + 5, y + 10)
        app.request('/k', c='A', cmd=1, wait=1)
        app.click_id('inspect')
        state = json.loads(next(w['t'] for w in app.snap() if w['i'] == 'result'))
        (root / f'{name}-selection.json').write_text(json.dumps(state, ensure_ascii=False, indent=2))
        assert state['original_html_preserved'], state
        assert state['diagrams'] == diagrams, state
        assert state['copy'] == state['text'], state
        for text in ['Before 中文 bold.', expected_source, 'After link.', 'fn main() {}']:
            assert text in state['copy'], (text, state)
        assert '<chatdiagram>' not in state['copy'] and '6d65726d616964:' not in state['copy'], state
        if 'EditedOS' in expected_source:
            assert 'GrapheneOS' not in state['copy'], state
        snapshot = app.request('/snap', all=1)['s']
        (root / f'{name}-snapshot.json').write_text(json.dumps(snapshot, ensure_ascii=False, indent=2))
        rendered = [w for w in snapshot if w['ty'] == 'MarkdownDiagram' and w['r'][2] > 0]
        assert len(rendered) == diagrams, rendered
        if rendered:
            width = next(w['r'][2] for w in snapshot if w['i'] == 'content')
            for item in rendered:
                assert 10 < item['r'][2] <= width + 1 and item['r'][3] > 10, item
        report['checks'].append(name)

    try:
        for _ in range(120):
            assert app.process.poll() is None, 'Native fixture exited during startup'
            try:
                if app.request('/s')['pid'] == app.process.pid:
                    break
            except OSError:
                pass
            time.sleep(.25)
        app.wait_text('Ready')
        inspect('grapheneos', source, 1)
        app.click_id('edit')
        inspect('edited', source.replace('GrapheneOS', 'EditedOS'), 1)
        app.click_id('invalid')
        inspect('invalid-source-fallback', 'not a diagram\n中文 remains visible', 0)
        app.click_id('valid')
        app.click_id('narrow')
        inspect('narrow', source, 1)
        app.click_id('restyle')
        inspect('restyled', source, 1)
        log = (app.output / 'native.log').read_text(errors='replace')
        assert 'panicked at' not in log and 'not found' not in log, log
        report['passed'] = True
    finally:
        app.stop()
        (root / 'result.json').write_text(json.dumps(report, indent=2))
        print(root / 'result.json')


if __name__ == '__main__':
    main()
