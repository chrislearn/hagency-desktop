#!/usr/bin/env python3
"""Exercise the real creation wizard in isolated, hidden Makepad windows.

No Matrix client, live room creation, credentials or model calls. Pixel captures
and interaction checks support a separate visual review; they do not assign a score.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import time
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
CASES = {
    'zh-desktop': ('zh-CN', 720, 780, False, False),
    'en-desktop': ('en', 720, 780, False, False),
    'zh-compact': ('zh-CN', 390, 844, False, False),
    'en-compact': ('en', 390, 844, False, False),
    'zh-dark': ('zh-CN', 720, 780, True, False),
    'en-dark-compact': ('en', 390, 844, True, False),
    'zh-large': ('zh-CN', 390, 844, False, True),
    'en-large-dark': ('en', 390, 844, True, True),
}

class Review:
    def __init__(self, binary, output, case, profile=None):
        language, self.width, self.height, dark, large = case
        self.language = language
        self.output = output.resolve()
        self.output.mkdir(parents=True, exist_ok=True)
        profile = (profile or self.output / 'profile').resolve()
        self.profile = profile
        profile.mkdir(exist_ok=True)
        (profile / 'ui-language.json').write_text(json.dumps(language))
        self.checks, self.trace = [], []
        self.binary_sha256 = hashlib.sha256(binary.read_bytes()).hexdigest()
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            self.port = sock.getsockname()[1]
        env = dict(os.environ, HAGENCY_DESKTOP_DATA_DIR=str(profile),
                   MAKEPAD_REMOTE=str(self.port), MAKEPAD_HIDE_WINDOWS='1', MAKEPAD_NO_FOCUS='1',
                   WIZARD_REVIEW_WIDTH=str(self.width), WIZARD_REVIEW_HEIGHT=str(self.height))
        env.pop('MAKEPAD_FOCUS', None)
        self.log = (self.output / 'native.log').open('w')
        self.process = subprocess.Popen([str(binary.resolve())], cwd=ROOT, env=env,
                                        stdout=self.log, stderr=subprocess.STDOUT)
        for _ in range(100):
            if self.process.poll() is not None:
                raise RuntimeError('Offline review exited; inspect native.log')
            try:
                if self.request('/s')['pid'] == self.process.pid:
                    break
            except OSError:
                time.sleep(.2)
        else:
            raise RuntimeError('Review window did not become ready')
        self.request('/event', data='theme:dark' if dark else 'theme:light')
        if large:
            self.request('/event', data='theme:large-text')
        time.sleep(.2)

    def request(self, path, **params):
        self.trace.append({'path': path, 'params': params})
        url = f'http://127.0.0.1:{self.port}{path}?' + urllib.parse.urlencode(params)
        with urllib.request.urlopen(url, timeout=15) as response:
            return json.load(response)

    def snap(self):
        return self.request('/snap', w=0)['s']

    def widget(self, ident):
        return next(w for w in self.snap() if w.get('i') == ident and w.get('r', [0,0,0,0])[2] > 0)

    def reveal(self, ident):
        for _ in range(10):
            sx, sy, sw, sh = self.widget('create_form')['r']
            try:
                widget = self.widget(ident)
                x, y, width, height = widget['r']
                if sy <= y+height/2 < sy+sh:
                    return widget
                dy = y+height/2-(sy+sh/2)
            except StopIteration:
                dy = max(80, sh*.6)
            self.request('/m', w=0, k='scroll', x=sx+sw/2, y=sy+sh/2,
                         dy=dy, precise=1, wait=1)
            time.sleep(.25)
        raise AssertionError((ident, 'not reachable in scroll viewport'))

    def click(self, ident):
        widget = (self.reveal(ident) if ident in
                  ('ordinary_kind', 'agent_kind', 'room_kind', 'name', 'topic', 'public', 'encrypted')
                  else self.widget(ident))
        assert widget.get('enabled', True), ident
        x, y, width, height = widget['r']
        self.request('/click', w=0, x=x+width/2, y=y+height/2, wait=1)
        time.sleep(.15)

    def fill(self, ident, value):
        self.click(ident)
        self.request('/k', w=0, c='A', cmd=1, wait=1)
        self.request('/k', w=0, c='Backspace', wait=1)
        if value:
            self.request('/t', w=0, t=value, wait=1)
        time.sleep(.1)

    def capture(self, name, window=0):
        image = self.request('/g', w=window, scale=1)
        assert image.get('capture_kind') == 'pixels'
        shutil.copy2(image['png'], self.output / (name + '.png'))
        (self.output / (name + '.json')).write_text(json.dumps(self.request('/snap', w=window)['s'], ensure_ascii=False, indent=2))

    def footer(self, ident):
        x, y, width, height = self.widget(ident)['r']
        assert height >= 44, (ident, height)
        assert 0 <= x and x+width <= self.width+1 and 0 <= y and y+height <= self.height+1, (ident, x,y,width,height)

    def run(self):
        for kind, name in enumerate(['ordinary', 'agent', 'room']):
            self.request('/event', data='wizard:0:0:ready')
            time.sleep(.15)
            self.click(name + '_kind')
            self.footer('wizard_next')
            self.capture(name + '-choice')
            self.click('wizard_next')
            self.footer('create')
            expected = (['新建普通空间', '新建 Agent 空间', '新建聊天室'] if self.language == 'zh-CN'
                        else ['New ordinary space', 'New Agent space', 'New chat room'])[kind]
            assert self.widget('wizard_title')['t'] == expected, 'The card must select the requested type'
            self.fill('name', '')
            assert not self.widget('create').get('enabled', True), 'Empty names must not be submitted'
            self.fill('name', '设计协作' if name != 'room' else '产品讨论')
            assert self.widget('create').get('enabled', False), 'A valid configured draft must be submittable'
            self.capture(name + '-details')
            if name == 'room':
                before = self.reveal('encrypted')['c']
                self.click('encrypted')
                assert self.widget('encrypted')['c'] != before, 'Encryption switch must respond'
                self.click('encrypted')
                assert self.widget('encrypted')['c'] == before, 'Encryption choice must be retained'
                sx, sy, sw, sh = self.widget('create_form')['r']
                self.request('/m', w=0, k='scroll', x=sx+sw/2, y=sy+sh/2, dy=2000, precise=1, wait=1)
                time.sleep(.3)
                self.capture(name + '-privacy')
                self.footer('create')
            self.click('wizard_back')
            self.click('wizard_next')
            assert self.widget('name').get('val'), 'Back must retain the draft'
            self.checks.append(name + ': whole-card selection, name validation, sticky action, retained draft')
        for state in ['ready', 'pending']:
            self.request('/event', data='wizard:1:2:' + state)
            time.sleep(.15)
            if state == 'pending':
                self.footer('finish_setup')
                assert not any(w.get('i') == 'create' and w.get('r', [0,0,0,0])[2] > 0 for w in self.snap())
            else:
                self.footer('create_child')
            self.capture('completion-' + state)
            self.checks.append('completion ' + state + ': clear next action and no duplicate creation')

    def close(self, passed):
        if not passed:
            try:
                self.capture('failure')
            except Exception:
                pass
        (self.output / 'report.json').write_text(json.dumps({'passed': passed, 'renderer': 'hidden native Makepad / Metal', 'windows_hidden': True, 'profile': str(self.profile), 'binary_sha256': self.binary_sha256, 'checks': self.checks, 'trace': self.trace}, indent=2))
        try:
            self.request('/quit')
        except OSError:
            pass
        try:
            self.process.wait(timeout=8)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            self.process.wait(timeout=8)
        self.log.close()

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, default=ROOT/'target/debug/examples/space_creation_review')
    parser.add_argument('--output', type=Path, default=ROOT/'.run/space-wizard-ios')
    parser.add_argument('--case', choices=list(CASES), action='append')
    args = parser.parse_args()
    for name in args.case or CASES:
        app = Review(args.binary, args.output/name, CASES[name])
        passed = False
        try:
            app.run()
            passed = True
            print(name, 'PASS', len(app.checks), flush=True)
        finally:
            app.close(passed)

if __name__ == '__main__':
    main()
