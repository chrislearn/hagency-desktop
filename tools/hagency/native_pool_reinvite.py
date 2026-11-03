#!/usr/bin/env python3
"""Offline native reauthorization and reinvitation warning acceptance."""
import argparse
import hashlib
import json
from pathlib import Path

from native_local_agents import Host, ROOT
from native_recurring_periods import CALENDAR, PERIOD


def inside(host, name):
    row=host.row(name)
    x,y,width,height=row['r']
    window_width,window_height=host.req('/s')['w'][0]['sz']
    assert x>=0 and y>=0 and x+width<=window_width+1 and y+height<=window_height+1,(name,row['r'])
    return row


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--binary',type=Path,default=ROOT/'target/debug/examples/hagency_visual')
    args=parser.parse_args()
    results=[]
    source=ROOT/'apps/hagency/bundle/main.splash'
    before=hashlib.sha256(source.read_bytes()).hexdigest()
    for language in ('en','zh-CN'):
        for narrow in (True,False):
            label=language+('-narrow' if narrow else '-wide')
            for recovered in (False,True):
                case='reinvite-warning' if recovered else 'reauthorize-confirmation'
                output=(args.output/label/case).resolve()
                request=dict(calendar=CALENDAR,renewalMode='owner' if recovered else None,
                    currentPeriod=dict(PERIOD,state='available',consumedTokens=0,remainingTokens=1000000) if recovered else None,
                    canRenew=False,canReallocate=not recovered,currentGrantRevision=8,
                    state='approved_waiting_executor' if recovered else 'suspended',
                    periodResetLabel='2026-11-09 00:00 PST · America/Los_Angeles',
                    blockingReason=None if recovered else 'agent_period_ended')
                if recovered:request['roomsRequiringReinvite']=2
                host=Host(args.binary.resolve(),output,language,narrow,workspace='owner',
                    account='@dspfac:example.test',pool=request)
                try:
                    host.click('inbox',content=False);host.click('owner_history');host.click('owner_request_open')
                    if recovered:
                        host.reveal('pool_rooms_reinvite')
                        warning=inside(host,'pool_rooms_reinvite')['t']
                        assert '2' in warning
                        assert ('fresh invitation' if language=='en' else '重新邀请') in warning
                        assert ('Previous removals are respected' if language=='en' else '之前的移出操作会保留') in warning
                        host.capture('warning')
                    else:
                        host.click('pool_agent_reallocate')
                        host.wait(lambda:host.row('reallocate_confirm'),'reauthorization confirmation')
                        assert not inside(host,'reallocate_confirm')['enabled']
                        before_button=inside(host,'reallocate_cancel')['r']
                        words='\n'.join(row.get('t','') for row in host.snap() if row.get('r',[0,0,0,0])[2]>0)
                        assert ('Rooms the Agent left may need a fresh invitation' if language=='en' else '已退出的房间可能需要重新邀请') in words
                        host.capture('choose-renewal')
                        host.click('reallocate_owner')
                        assert inside(host,'reallocate_confirm')['enabled']
                        assert inside(host,'reallocate_cancel')['r']==before_button
                        host.capture('owner-renewal')
                    results.append(dict(language=language,narrow=narrow,case=case,passed=True))
                finally:host.close()
    assert hashlib.sha256(source.read_bytes()).hexdigest()==before,'Source changed during native review'
    args.output.mkdir(parents=True,exist_ok=True)
    (args.output/'report.json').write_text(json.dumps(dict(cases=results,offline=True,sourceSha256=before),indent=2))
    print(json.dumps(results))


if __name__=='__main__':main()
