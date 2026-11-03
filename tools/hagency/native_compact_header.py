#!/usr/bin/env python3
"""Headless production widgets: compact navigation and legacy pool periods.

Only isolated profiles and fixture services are used; no live grants are changed.
"""
import argparse
import json
from pathlib import Path
from native_local_agents import Host, ROOT


def owner_fixtures(calendar=None):
    now = 1793656687934
    owner = '@dspfac:example.test'
    rows, connections, pools, details = [], [], [], []
    for index, model in enumerate(('gpt-6.1-terra', 'gpt-6-astra')):
        rid, cid = f'resource_{index}', f'pool_{index}'
        row = dict(id=rid, model=model, framework='codex', reasoning='high',
                   capabilities=['coding'], published=False, localName=dict(name='', revision=1),
                   budget=dict(remainingTokens=20000000, pool=dict(ceiling=20000000, committed=0, period='monthly'), seat=dict(quota=None)),
                   usage=dict(spent=0, spend_period_key='2026-11'))
        detail = dict(protocol='hagency-management-v1', resource=row, observedAtMs=now,
                      editing=dict(allowed=False, reason='previously_offered'), choices=[],
                      ceiling=dict(tokens=20000000, period='monthly'),
                      ceilingAmendment=dict(allowed=True, reason=None, sequence=0, minimumTokens=1, perRequestTokens=None))
        if calendar:
            detail['calendar'] = calendar
        rows.append(row)
        details.append(dict(when=dict(id=rid), response=detail))
        connections.append(dict(id=cid, label=model, resourceId=rid, ownerMxid=owner,
                                coordinatorMxid=owner, executorDeviceId='offline-device', state='admitted'))
        pools.append(dict(id=cid, ownerMxid=owner, publishedTokens=None, grantRevision=None, calendar=calendar))
    return {
        'hagency.accounts.list': dict(protocol='hagency-management-v1', items=[], nextAfter=None),
        'hagency.resources.list': dict(resources=rows, nextAfter=None),
        'hagency.resources.get': dict(_cases=details),
        'hagency.connections.list': dict(items=connections, nextAfter=None, observedAtMs=now),
        'hagency.pools.list': dict(items=pools, nextAfter=None, observedAtMs=now),
        'hagency.onboarding.status': dict(grantReviewAvailable=True),
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--binary', type=Path, default=ROOT/'target/release/examples/hagency_visual')
    args = parser.parse_args()
    results = []
    for language in ('en', 'zh-CN'):
        for narrow in (False, True):
            for workspace in ('owner', 'manager'):
                name = f'{language}-{"narrow" if narrow else "wide"}-{workspace}'
                out = args.output.resolve()/name
                out.mkdir(parents=True, exist_ok=True)
                host = Host(args.binary.resolve(), out, language, narrow, workspace=workspace,
                            account='@dspfac:example.test', fixture_overrides=owner_fixtures())
                try:
                    host.wait(lambda:host.row('entry_switch'), 'compact toolbar')
                    host.wait(lambda:not any(w.get('i')=='heading' for w in host.snap()), 'duplicate heading hidden')
                    width, _ = host.req('/s')['w'][0]['sz']
                    controls = ['accounts', 'resources', 'inbox'] if workspace=='owner' else ['overview','agents','projects','work']
                    boxes = [host.row(i)['r'] for i in controls+['entry_switch','refresh']]
                    assert all(x>=0 and x+w<=width and h>=44 for x,y,w,h in boxes), boxes
                    assert len({y for x,y,w,h in boxes}) == 1, boxes
                    host.capture('toolbar')
                    if workspace == 'owner':
                        host.click('resources', content=False)
                        host.wait(lambda:len([w for w in host.snap() if w.get('t','').startswith(('Publish ·','发布额度 ·'))])==2, 'distinct publication prompts')
                        host.capture('pools')
                        host.click('resource_detail')
                        host.reveal('resource_period_value')
                        host.wait(lambda:host.row('resource_period_value')['t'] in ('Not set','未设置'), 'legacy period shown')
                        host.reveal('resource_period_choose')
                        host.capture('period')
                        assert not any(w.get('i')=='accounts' for w in host.snap()), 'detail repeats primary navigation'
                        host.click('resource_back', content=False)
                        host.wait(lambda:host.row('entry_switch'), 'back restores primary navigation')
                    host.click('entry_switch', content=False)
                    host.wait(lambda:host.row('heading'), 'role chooser heading')
                    assert not any(w.get('i')=='navigation' for w in host.snap())
                    host.capture('roles')
                    results.append(dict(case=name, passed=True, toolbarHeight=50))
                finally:
                    host.close()
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output/'report.json').write_text(json.dumps(results, indent=2))
    print(json.dumps(results))


if __name__ == '__main__':
    main()
