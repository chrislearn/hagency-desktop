#!/usr/bin/env python3
"""Offline native publication refusal for an incompatible shared-account cap."""
import argparse
import hashlib
import json
from pathlib import Path

from native_local_agents import ROOT
from native_recurring_publication import preview_host

MESSAGE='A shared account limit requires all its pools to use UTC and the same period. Review the account limit or use a compatible pool.'


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--binary',type=Path,default=ROOT/'target/debug/examples/hagency_visual')
    args=parser.parse_args()
    translations=json.loads((ROOT/'resources/i18n/zh-CN.json').read_text())
    calendar=dict(cadence='monthly',timeZone='America/Los_Angeles')
    fixture=dict(panel='grant',state='publish',total=10000000,remaining=None,
        proposed=2000000,accountLimit=15000000,blockedReason='account_period_mismatch',
        managers=['@chris:test'],calendar=calendar,boundCalendar=calendar)
    results=[]
    for language in ('en','zh-CN'):
        for narrow in (True,False):
            output=(args.output/(language+('-narrow' if narrow else '-wide'))).resolve()
            host=preview_host(args.binary,output,language,narrow,fixture)
            try:
                host.event('ux:native-panel')
                expected=MESSAGE if language=='en' else translations[MESSAGE]
                host.wait(lambda:host.row('problem')['t']==expected,'explicit account calendar refusal')
                assert host.row('title')['t']==('Cannot Publish' if language=='en' else '无法发布')
                assert not host.row('primary')['enabled']
                host.reveal('problem')
                width,height=host.req('/s')['w'][0]['sz']
                for name in ('problem','primary','cancel'):
                    x,y,w,h=host.row(name)['r']
                    assert x>=0 and y>=0 and x+w<=width+1 and y+h<=height+1,(name,[x,y,w,h])
                if narrow:assert host.row('problem')['r'][3]>35,'Narrow refusal must wrap rather than clip'
                host.capture('refusal')
                results.append(dict(language=language,narrow=narrow,passed=True))
            finally:host.close()
    args.output.mkdir(parents=True,exist_ok=True)
    (args.output/'report.json').write_text(json.dumps(dict(cases=results,offline=True,
        fixture=fixture,grantUiSha256=hashlib.sha256((ROOT/'src/miniapps/grant_ui.rs').read_bytes()).hexdigest()),indent=2))
    print(json.dumps(results))


if __name__=='__main__':main()
