#!/usr/bin/env python3
"""Real native Invite members controls, with isolated offline completions."""
import argparse
import time
from urllib.error import HTTPError
from pathlib import Path
from native_review import Review, CASES

class InviteReview(Review):
    def capture(self, name, window=0):
        # A hidden Metal window can be between drawable frames. Retry only the
        # capture, never an interaction or a failed state assertion.
        for attempt in range(3):
            try:
                return super().capture(name, window)
            except HTTPError as error:
                if error.code != 404 or attempt == 2:
                    raise
                time.sleep(.3)

    def scenario(self, name):
        self.request('/event', data='invite:' + name)
        time.sleep(.15)

    def status(self):
        return self.widget('status_label')['t']

    def enabled(self):
        return self.widget('confirm_button').get('enabled', True)

    def open(self):
        self.click('invite_people')
        assert self.widget('title')['t'] == ('邀请加入 Team room' if self.language == 'zh-CN' else 'Invite to Team room')
        self.footer('cancel_button')
        self.footer('confirm_button')

    def closed(self):
        assert not any(w.get('i') == 'user_id_input' and w.get('r', [0,0,0,0])[2] > 0 for w in self.snap())

    def run(self):
        self.request('/event', data='picker:members')
        time.sleep(.15)
        self.open()
        self.click('confirm_button')
        assert self.status() == ('请输入名称或 Matrix ID。' if self.language == 'zh-CN' else 'Enter a name or Matrix ID.')
        self.capture('empty-input')
        self.fill('user_id_input', '@invalid')
        self.click('confirm_button')
        assert ('无效' in self.status() if self.language == 'zh-CN' else 'Invalid' in self.status())
        self.capture('invalid-id')
        self.fill('user_id_input', '白菜')
        self.click('confirm_button')
        assert self.widget('user_id_input')['val'] == '@_hagency_agt_cabbage:example.invalid'
        self.click('confirm_button')
        assert not self.enabled()
        assert self.status() == ('正在将 Agent 添加到此聊天室…' if self.language == 'zh-CN' else 'Adding Agent to this room…')
        for stale in ['stale','other-room']:
            before=self.status()
            self.scenario(stale)
            assert self.status() == before and not self.enabled()
        self.scenario('slow')
        self.capture('waiting')
        assert not self.enabled()
        self.click('cancel_button')
        self.closed()
        self.checks.append('Member button opens dialog; empty/invalid input shows errors; partial name resolves; stale results ignored; waiting can close')
        self.open()
        self.fill('user_id_input', '@_hagency_agt_cabbage:example.invalid')
        self.click('confirm_button')
        self.scenario('agent_service_invite_permission_required')
        assert self.enabled()
        assert ('权限' in self.status() if self.language == 'zh-CN' else 'permission' in self.status())
        self.capture('permission-error')
        self.click('confirm_button')
        self.scenario('pending')
        assert self.enabled()
        assert self.widget('confirm_button')['t'] == ('检查状态' if self.language == 'zh-CN' else 'Check status')
        self.capture('pending')
        self.click('confirm_button')
        self.scenario('success')
        self.footer('okay_button')
        assert self.status() == ('Agent 已加入此聊天室。在群聊中，请 @ Agent 让它回答。' if self.language == 'zh-CN' else 'Agent joined this room. In group chats, @ the Agent to ask it to reply.')
        self.capture('joined')
        self.click('okay_button')
        self.closed()
        self.checks.append('Permission error releases busy state; saved pending can check status; confirmed join shows Done')
        self.open()
        self.fill('user_id_input', '@friend:example.invalid')
        self.click('confirm_button')
        self.scenario('human')
        assert self.status() == ('邀请已发送，等待对方加入。' if self.language == 'zh-CN' else 'Invitation sent. Waiting for them to join.')
        self.capture('human-invited')
        self.click('okay_button')
        self.closed()
        self.checks.append('Human invite is clearly waiting for acceptance')
        log=(self.output/'native.log').read_text()
        for invalid in ['Expected expression','property crate not defined','panicked at']:
            assert invalid not in log, invalid

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--case',choices=CASES,required=True)
    args=parser.parse_args()
    review=InviteReview(args.binary,args.output,CASES[args.case])
    passed=False
    try:
        review.run()
        passed=True
    finally:
        review.close(passed)

if __name__ == '__main__':
    main()
