#!/usr/bin/env python3
"""Click Invite members from creation completion in the real App overlay stack.

Use hidden native Makepad/Metal windows, never a simplified dialog-only mock.
Check clicks and text input in addition to snapshots: a covered modal still
appears in accessibility metadata, which alone cannot detect this regression.
"""
import argparse
import time
from pathlib import Path
from native_review import CASES
from member_invite_review import InviteReview


class WizardInviteReview(InviteReview):
    def open(self):
        self.click('invite_created')
        title = '邀请加入 Team space' if self.language == 'zh-CN' else 'Invite to Team space'
        assert any(w.get('i') == 'title' and w.get('t') == title for w in self.snap())
        self.footer('cancel_button')
        self.footer('confirm_button')

    def completion(self, kind):
        expected = (['空间已创建', '空间已创建', '聊天室已创建'] if self.language == 'zh-CN'
                    else ['Your space is ready', 'Your space is ready', 'Your room is ready'])[kind]
        assert self.widget('wizard_title')['t'] == expected
        self.footer('invite_created')

    def run(self):
        # RinxContent is lazy: wait for its first draw before addressing children.
        for _ in range(100):
            if any(w.get('i') == 'login_screen' for w in self.snap()):
                break
            time.sleep(.1)
        else:
            raise AssertionError('Production RinxContent did not draw')

        for kind, name in enumerate(['ordinary', 'agent', 'room']):
            self.request('/event', data=f'completed:{kind}')
            time.sleep(.2)
            self.completion(kind)
            self.open()
            self.capture(name + '-invite-open')
            self.click('confirm_button')
            assert self.status() == (
                '请输入名称或 Matrix ID。' if self.language == 'zh-CN'
                else 'Enter a name or Matrix ID.'), 'Invitation must receive pointer events over the wizard'
            self.fill('user_id_input', '@invalid')
            assert self.widget('user_id_input')['val'] == '@invalid', 'Invitation must accept typing'
            self.click('confirm_button')
            assert ('无效' in self.status() if self.language == 'zh-CN' else 'Invalid' in self.status())
            self.capture(name + '-input-validation')
            self.click('cancel_button')
            self.closed()
            self.completion(kind)
            self.capture(name + '-cancel-return')
            self.checks.append(name + ': actual completion button opens an interactive foreground invitation; cancel preserves creation')

            self.open()
            assert self.widget('user_id_input')['val'] == '', 'Reopening must reset the form, keeping the same room'
            self.request('/k', w=0, c='Escape', wait=1)
            time.sleep(.2)
            self.closed()
            self.completion(kind)
            self.capture(name + '-escape-return')
            self.checks.append(name + ': Escape closes only the invitation; the same completed space/room can invite again')

            self.open()
            self.request('/k', w=0, c='Escape', wait=1)
            time.sleep(.2)
            self.closed()
            self.completion(kind)
            self.open()
            # A background click dismisses the child; it cannot activate the
            # wizard action underneath that same click.
            self.click('invite_created')
            self.closed()
            self.completion(kind)
            self.capture(name + '-background-return')
            self.request('/k', w=0, c='Escape', wait=1)
            time.sleep(.2)
            assert not any(w.get('i') == 'invite_created' and w.get('r', [0,0,0,0])[2] > 0 for w in self.snap())
            self.checks.append(name + ': repeated Escape closes child then parent; background click does not pass through')

        log = (self.output / 'native.log').read_text()
        for invalid in ['Expected expression', 'property crate not defined', 'panicked at', 'starting matrix sdk loop']:
            assert invalid not in log, invalid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--case', choices=CASES, action='append')
    args = parser.parse_args()
    for name in args.case or ['zh-desktop', 'en-compact', 'zh-dark', 'en-large-dark']:
        review = WizardInviteReview(args.binary, args.output / name, CASES[name])
        passed = False
        try:
            review.run()
            passed = True
            print(name, 'PASS', len(review.checks), flush=True)
        finally:
            review.close(passed)


if __name__ == '__main__':
    main()
