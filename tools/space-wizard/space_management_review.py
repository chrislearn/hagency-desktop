#!/usr/bin/env python3
"""Real Manage space controls with explicit offline registration completions.

Uses the production row rendering and keyboard activation. The ordinary Rust
regression tests exercise actual link-before-AdoptRoom effect ordering. No real
Matrix account, room creation, registration or Agent execution is performed.
"""
import argparse
import json
import time
from pathlib import Path
from native_review import Review, CASES

class ManagementReview(Review):
    def settle(self):
        previous = None
        for _ in range(10):
            self.request('/g', w=0)
            current = self.snap()
            if current == previous:
                return current
            previous = current
        raise AssertionError('Widget snapshot did not settle after ten rendered frames')

    def scroll_rooms(self, delta):
        x,y,w,h = self.widget('rooms')['r']
        self.request('/m', w=0, k='scroll', x=x+w/2, y=y+h/2, dy=delta, precise=1, wait=1)
        self.settle()

    def click(self, ident):
        super().click(ident)
        self.settle()

    def scenario(self, value):
        self.request('/event', data='manage:'+value, wait=1)
        self.settle()
        if value != 'focus-finish':
            self.scroll_rooms(-2000)
        (self.output/('state-'+value+'.json')).write_text(json.dumps(self.snap(),ensure_ascii=False,indent=2))

    def enabled(self, ident):
        return self.widget(ident).get('enabled', True)

    def visible(self, ident):
        return any(row.get('i')==ident and row['r'][2]>0 and row['r'][3]>0 for row in self.snap())

    def submissions(self):
        return [json.loads(line.split('space-registration-review ',1)[1]) for line in self.request('/log', n=500)['l'] if 'space-registration-review ' in line]

    def run(self):
        self.scenario('unknown')
        assert not self.enabled('toggle')
        assert self.visible('registration')
        assert not self.visible('finish_registration')
        self.capture('01-unknown-space-type')
        self.checks.append('Unknown space type blocks Add and explains Refresh')
        self.scenario('unlinked')
        assert self.enabled('toggle') and not self.visible('finish_registration')
        self.click('toggle')
        assert not self.enabled('toggle')
        first=self.submissions()[-1]
        assert first=={'parent':'!review-space:example.invalid','room':'!existing-room:example.invalid','project':'review-project','retry':False}
        self.capture('02-adding-original-room')
        self.checks.append('Add submits the existing room and Project identities once and disables repeat clicks')
        self.scenario('failure')
        assert self.enabled('finish_registration')
        self.scroll_rooms(2000)
        assert self.visible('toggle') and self.enabled('toggle')
        self.scroll_rooms(-2000)
        assert 'Agent' in self.widget('status')['t']
        self.capture('03-linked-registration-failed')
        self.scenario('reopen')
        assert self.enabled('finish_registration')
        assert self.widget('id')['t']==first['room']
        self.capture('04-reopened-missing-registration')
        self.checks.append('Failed registration leaves the same linked room visible; reopened server snapshot exposes Finish setup and Remove')
        self.scenario('focus-finish')
        self.request('/k', w=0, c='Return', wait=1)
        self.settle()
        second=self.submissions()[-1]
        assert second==dict(first,retry=True)
        assert not self.enabled('finish_registration')
        count=len(self.submissions())
        self.request('/k', w=0, c='Return', wait=1)
        self.settle()
        assert not self.enabled('finish_registration')
        assert len(self.submissions())==count
        self.checks.append('Keyboard Return retries the original room; busy state prevents a second submission')
        self.scenario('success')
        assert not self.visible('finish_registration') and not self.visible('registration')
        assert self.enabled('toggle') and self.widget('id')['t']==first['room']
        self.capture('05-registered-success')
        self.checks.append('Confirmed registration removes retry while keeping the original room and Remove action')
        self.scenario('inactive')
        assert not self.visible('finish_registration') and self.visible('registration')
        self.capture('06-inactive-registration')
        self.checks.append('Inactive registration is explicit and offers no unsupported reactivation')

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--case',choices=CASES,required=True)
    args=parser.parse_args()
    app=ManagementReview(args.binary,args.output,CASES[args.case]);passed=False
    try:
        time.sleep(.5)
        app.run();passed=True
    except Exception:
        app.capture('failure')
        raise
    finally:app.close(passed)
