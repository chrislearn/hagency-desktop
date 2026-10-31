#!/usr/bin/env python3
"""Actual native picker interactions with offline responses and hidden windows."""
import argparse
import time
from pathlib import Path
from native_review import Review, CASES

class AgentReview(Review):
    def scenario(self, name):
        self.request('/event', data='picker:' + name)
        time.sleep(.15)

    def enabled(self, ident):
        return self.widget(ident).get('enabled', True)

    def closed(self):
        assert not any(w.get('i') == 'close_agent_picker' for w in self.snap()), 'Close must dismiss the overlay'

    def run(self):
        self.scenario('ready')
        self.footer('close_agent_picker')
        self.footer('invite_selected_agent')
        self.capture('ready')
        assert self.enabled('invite_selected_agent')
        self.click('invite_selected_agent')
        assert not self.enabled('invite_selected_agent'), 'Double submission must be disabled'
        self.scenario('slow')
        self.capture('waiting')
        assert self.enabled('close_agent_picker'), 'Waiting must not trap the user'
        self.click('close_agent_picker')
        self.closed()
        self.click('invite_my_agent')
        assert not self.enabled('invite_selected_agent'), 'Reopen must retain the running request'
        self.checks.append('slow original request: duplicate disabled, close responds, reopen preserves progress')
        self.scenario('agent_service_invite_permission_required')
        assert self.enabled('invite_selected_agent'), 'A failed request must release the waiting state'
        assert ('权限' in self.widget('agent_status')['t'] if self.language == 'zh-CN' else 'permission' in self.widget('agent_status')['t'])
        self.capture('permission-error')
        self.click('invite_selected_agent')
        self.scenario('pending')
        assert self.enabled('invite_selected_agent')
        assert self.widget('invite_selected_agent')['t'] == ('检查状态' if self.language == 'zh-CN' else 'Check status')
        self.capture('pending')
        self.click('invite_selected_agent')
        self.scenario('success')
        assert not self.enabled('invite_selected_agent')
        assert self.widget('close_agent_picker')['t'] == ('完成' if self.language == 'zh-CN' else 'Done')
        self.capture('success')
        self.click('close_agent_picker')
        self.closed()
        self.checks.append('permission failure → same request retry → pending status → confirmed success → Done')
        self.scenario('ready')
        self.click('invite_selected_agent')
        self.scenario('malformed')
        assert self.enabled('invite_selected_agent'), 'Invalid response must not strand the busy state'
        self.capture('invalid-response')
        self.click('close_agent_picker')
        self.closed()
        self.checks.append('malformed completion releases waiting state and remains dismissible')

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--case', choices=CASES, required=True)
    args=parser.parse_args()
    review=AgentReview(args.binary,args.output,CASES[args.case])
    passed=False
    try:
        review.run()
        passed=True
    finally:
        review.close(passed)

if __name__ == '__main__':
    main()
