#!/usr/bin/env python3
"""Native mention selection, editing, draft restoration and wire identity checks."""
import argparse
import json
import time
from urllib.error import HTTPError
from pathlib import Path
from native_review import CASES
from member_invite_review import InviteReview

AGENT = '@_hagency_agent_yellow:example.invalid'
FIRST = '@first_alex:example.invalid'
SECOND = '@second_alex:example.invalid'


class MentionReview(InviteReview):
    def request(self, path, **params):
        try:
            return super().request(path, **params)
        except HTTPError as error:
            body = error.read().decode()
            if error.code == 404 and path in ['/click', '/k', '/t'] and json.loads(body).get('err') == 'requested input frame could not be submitted; retry':
                # Makepad already applied this input before its hidden Metal
                # presentation failed. Never replay the click/keystroke. Ask
                # only for a fresh rendered frame, then check the actual state.
                self.trace.append({'frame_ack_recovery': path, 'error': body})
                for attempt in range(3):
                    time.sleep(.2)
                    try:
                        frame = super().request('/g', w=0, scale=1)
                        assert frame.get('capture_kind') == 'pixels'
                        return {'ok': 1, 'frame_ack_recovered': True}
                    except HTTPError as capture_error:
                        if capture_error.code != 404 or attempt == 2:
                            raise
            if path != '/g':
                print(path, body, flush=True)
            raise

    def command(self, action):
        self.request('/event', data='mentions:' + action)
        time.sleep(.1)

    def key(self, code, **mods):
        self.request('/k', w=0, c=code, wait=1, **mods)
        time.sleep(.05)

    def type(self, text):
        self.request('/t', w=0, t=text, wait=1)
        time.sleep(.15)

    def inspect(self, expected):
        self.command('inspect')
        state = json.loads((self.profile / 'mention-review.json').read_text())
        actual = state['message'].get('m.mentions', {}).get('user_ids', [])
        assert sorted(actual) == sorted(expected), state
        assert 'matrix.to' not in state['visible'] and ':example.invalid' not in state['visible'], state
        return state

    def choose(self, query, index=0):
        self.type('@' + query)
        for _ in range(index):
            self.key('ArrowDown')
        self.key('ReturnKey')

    def clear(self):
        self.command('clear')
        self.click('text_input')

    def run(self):
        for _ in range(50):
            if any(w.get('i') == 'text_input' and w.get('r', [0,0,0,0])[2] > 0 for w in self.snap()):
                break
            time.sleep(.1)
        else:
            raise AssertionError('Composer did not draw')
        self.command('ready')
        self.click('text_input')
        self.type('@')
        labels = [w.get('t', '') for w in self.snap() if w.get('ty') == 'Label']
        assert '小黄' in labels and 'Chris' in labels
        assert not any(':example.invalid' in label or 'matrix.to' in label for label in labels)
        self.capture('name-only-picker')
        self.type('小黄')
        # Select through the actual popup row's pointer hit handler.
        title = next(w for w in self.snap() if w.get('i') == 'title' and w.get('t') == '小黄')
        x, y, width, height = title['r']
        self.request('/click', w=0, x=x+width/2, y=y+height/2, wait=1)
        time.sleep(.2)
        state = self.inspect([AGENT])
        assert state['visible'] == '@小黄 ', state
        assert state['focused'], state
        assert AGENT in state['message']['formatted_body']
        self.type('继续输入')
        assert self.inspect([AGENT])['visible'] == '@小黄 继续输入'
        self.key('Z', cmd=1)
        assert self.inspect([AGENT])['visible'] == '@小黄 '
        self.capture('agent-name-and-rendered-message')
        self.key('Z', cmd=1)
        self.inspect([])
        self.key('Z', cmd=1, shift=1)
        assert self.inspect([AGENT])['visible'] == '@小黄 '
        self.type('你好')
        self.command('roundtrip')
        assert self.inspect([AGENT])['visible'] == '@小黄 你好'
        self.capture('edited-message-restored')
        self.checks.append('Popup shows names only; pointer selection renders @name; wire keeps agent ID; undo/redo and message editing preserve identity')

        self.clear()
        self.choose('Alex')
        self.choose('Alex', 1)
        assert self.inspect([FIRST, SECOND])['visible'] == '@Alex @Alex '
        self.command('save')
        self.clear()
        self.choose('Chris')
        self.inspect(['@chris:example.invalid'])
        self.command('restore')
        assert self.inspect([FIRST, SECOND])['visible'] == '@Alex @Alex '
        self.click('text_input')
        self.key('ArrowLeft', cmd=1)
        for _ in range(6):
            self.key('ArrowRight', shift=1)
        self.key('Backspace')
        assert self.inspect([SECOND])['visible'] == '@Alex '
        self.key('Z', cmd=1)
        self.inspect([FIRST, SECOND])
        self.key('Z', cmd=1, shift=1)
        self.inspect([SECOND])
        self.capture('same-name-identity-after-delete')
        self.checks.append('Keyboard selection distinguishes same-name users; switching drafts and deleting/undoing the first mention never targets the wrong user')

        self.clear()
        self.choose('小黄')
        self.key('A', cmd=1)
        self.key('Backspace')
        self.type('@小黄 ')
        self.inspect([])
        self.clear()
        self.type('/plain ')
        self.choose('小黄')
        state = self.inspect([AGENT])
        assert state['message']['body'] == '@小黄' and 'formatted_body' not in state['message']
        self.clear()
        self.type('/spoiler ')
        self.choose('Chris')
        state = self.inspect(['@chris:example.invalid'])
        assert 'matrix.to' not in state['message']['formatted_body']
        self.capture('literal-formatting-keeps-short-names')
        self.checks.append('Retyping a deleted name does not resurrect a mention; plain and spoiler formatting never prints hidden IDs')
        log = (self.output / 'native.log').read_text()
        assert 'panicked at' not in log and 'Expected expression' not in log


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--case', choices=CASES, action='append')
    args = parser.parse_args()
    for name in args.case or ['zh-desktop', 'en-compact', 'zh-dark', 'en-large-dark']:
        review = MentionReview(args.binary, args.output / name, CASES[name])
        passed = False
        try:
            review.run()
            passed = True
            print(name, 'PASS', len(review.checks), flush=True)
        finally:
            review.close(passed)


if __name__ == '__main__':
    main()
