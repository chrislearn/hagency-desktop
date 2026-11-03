#!/usr/bin/env python3
"""Real offline native allocation and periodic authorization screens, EN/ZH."""
import argparse
import hashlib
import json
from pathlib import Path
from native_pool_application import App
from native_local_agents import Host, ROOT

CALENDAR = dict(cadence="weekly", timeZone="America/Los_Angeles")
PERIOD = dict(key="2026-11-02", startMs=1793606400000, endMs=1794211200000,
              tokensPerPeriod=1000000, consumedTokens=1000000, remainingTokens=0,
              state="exhausted", nextResetMs=1794211200000)

def assert_single_row(first, second):
    ax, ay, aw, ah = first['r']
    bx, by, bw, bh = second['r']
    assert ay == by and ax + aw <= bx and ah >= 44 and bh >= 44, (first, second)


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--binary', type=Path, default=ROOT/'target/release/examples/hagency_visual')
    args=parser.parse_args(); results=[]
    for language in ('en','zh-CN'):
        for narrow in (True,False):
            label=language+('-narrow' if narrow else '-wide')
            output=args.output/label/'allocation'; output.mkdir(parents=True,exist_ok=True)
            app=App(output.resolve(),args.binary.resolve(),language,narrow,calendar=CALENDAR)
            try:
                app.click('pool_quarter')
                assert not app.row('pool_review')['enabled']
                app.click('pool_renew_automatic')
                assert app.row('pool_review')['enabled']
                assert json.loads(app.state_path.read_text())['poolDraft']['renewalMode']=='automatic'
                # /snap reports the clipped hit rectangle at the viewport edge.
                # Reveal the whole control before checking its touch target.
                app.req('/m',k='scroll',x=200,y=500,dy=160,wait=1)
                assert_single_row(app.row('pool_renew_automatic'), app.row('pool_renew_owner'))
                app.visible('pool_renew_owner');app.shot('automatic')
                app.click('pool_renew_owner');app.click('pool_review')
                app.visible('pool_submit');app.req('/m',k='scroll',x=200,y=500,dy=-10000,wait=1);app.shot('review')
                draft=json.loads(app.state_path.read_text())['poolDraft']
                assert draft['renewalMode']=='owner' and draft['pool']['calendar']==CALENDAR
                results.append(dict(case=label+' allocation',passed=True))
            finally:app.stop()
            output=args.output/label/'compact-header'
            host=Host(args.binary.resolve(),output.resolve(),language,narrow,pool=dict(
                definition=dict(name='小青', description='编程助手', capability='architect'),
                calendar=CALENDAR,renewalMode='automatic',currentPeriod=dict(PERIOD,state='active',consumedTokens=0),
                runtimeState='qualified',blockingReason=None))
            try:
                host.click('agents',content=False);host.click('pool_agent_open')
                host.wait(lambda:host.row('pool_agent_start'), 'response settings')
                # Plain layout Views have no hit area in /snap. Measure the
                # visible title, status and action instead of an empty parent.
                boxes=[host.row(i)['r'] for i in ('pool_agent_name','pool_agent_header_state','pool_agent_start')]
                height=max(y+h for x,y,w,h in boxes)-min(y for x,y,w,h in boxes)
                assert height<=86, boxes
                assert host.row('pool_agent_start')['r'][3]>=44
                host.capture('header')
                host.event('ux:large-text')
                host.wait(lambda:host.row('pool_agent_start'), 'large text response settings')
                host.capture('large-text')
                results.append(dict(case=label+' compact header',passed=True,height=height))
            finally:host.close()
            for period_state in ('exhausted','awaiting_executor'):
                output=args.output/label/period_state
                period=dict(PERIOD,state=period_state)
                if period_state=='awaiting_executor':period['remainingTokens']=None
                host=Host(args.binary.resolve(),output.resolve(),language,narrow,pool=dict(calendar=CALENDAR,
                    renewalMode='automatic',currentPeriod=period,canRenew=False,
                    periodResetLabel='2026-11-09 00:00 PST · America/Los_Angeles',
                    blockingReason='agent_period_'+period_state,runtimeState='qualified'))
                try:
                    host.click('agents',content=False);host.click('pool_agent_open')
                    host.capture('status')
                    host.reveal('pool_period_remaining');host.capture('period')
                    host.reveal('pool_period_reset');host.capture('reset')
                    host.wait(lambda: host.row('pool_period_reset'),'next reset')
                    assert 'America/Los_Angeles' in host.row('pool_period_reset')['t']
                    assert not any(w.get('i')=='pool_agent_renew' and w['r'][2]>0 for w in host.snap())
                    if period_state=='awaiting_executor':
                        assert host.row('pool_period_remaining')['t']==('Remaining allowance awaiting usage confirmation' if language=='en' else '剩余额度待用量确认')
                    results.append(dict(case=label+' '+period_state,passed=True))
                finally:host.close()
            for recovery in (False,True):
                output=args.output/label/('reauthorize' if recovery else 'owner-renew')
                period=dict(PERIOD,state='awaiting_owner',consumedTokens=0)
                request=dict(calendar=CALENDAR,renewalMode=None if recovery else 'owner',
                    currentPeriod=None if recovery else period,canRenew=not recovery,canReallocate=recovery,
                    currentGrantRevision=8,state='suspended' if recovery else 'approved_waiting_executor',
                    periodResetLabel='2026-11-09 00:00 PST · America/Los_Angeles',
                    blockingReason='agent_period_ended' if recovery else 'agent_period_awaiting_owner')
                host=Host(args.binary.resolve(),output.resolve(),language,narrow,workspace='owner',
                          account='@dspfac:example.test',pool=request)
                try:
                    host.click('inbox',content=False);host.click('owner_history');host.click('owner_request_open')
                    control='pool_agent_reallocate' if recovery else 'pool_agent_renew'
                    host.reveal(control);host.capture('action')
                    host.click(control)
                    if recovery:
                        host.wait(lambda:host.row('reallocate_confirm'),'reallocate confirmation')
                        assert not host.row('reallocate_confirm')['enabled']
                        host.click('reallocate_owner')
                        assert host.row('reallocate_confirm')['enabled']
                        assert_single_row(host.row('reallocate_automatic'),host.row('reallocate_owner'))
                    else:host.wait(lambda:host.row('pool_renew_confirm'),'renew confirmation')
                    host.capture('confirmation')
                    results.append(dict(case=label+(' reauthorize' if recovery else ' owner renew'),passed=True))
                finally:host.close()
    args.output.mkdir(parents=True,exist_ok=True)
    (args.output/'report.json').write_text(json.dumps(dict(cases=results,offline=True,
       sourceSha256=hashlib.sha256((ROOT/'apps/hagency/bundle/main.splash').read_bytes()).hexdigest()),indent=2))
    print(json.dumps(results))
if __name__=='__main__':main()
