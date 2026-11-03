#!/usr/bin/env python3
"""Production Makepad native calendar choices in isolated read-only previews."""
import argparse
import json
import os
from pathlib import Path
from native_local_agents import Host, ROOT

def preview_host(binary,output,language,narrow,fixture):
    output.mkdir(parents=True,exist_ok=True)
    preview=output/'native-panel.json';preview.write_text(json.dumps(fixture))
    before=os.environ.get('HAGENCY_NATIVE_PANEL');os.environ['HAGENCY_NATIVE_PANEL']=str(preview)
    try:return Host(binary.resolve(),output,language,narrow)
    finally:
        if before is None:os.environ.pop('HAGENCY_NATIVE_PANEL',None)
        else:os.environ['HAGENCY_NATIVE_PANEL']=before

def main():
    p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True)
    p.add_argument('--binary',type=Path,default=ROOT/'target/release/examples/hagency_visual');a=p.parse_args()
    reports=[]
    for language in ('en','zh-CN'):
        for narrow in (True,False):
            output=(a.output/(language+('-narrow' if narrow else '-wide'))).resolve();output.mkdir(parents=True,exist_ok=True)
            fixture=dict(panel='grant',state='publish',total=10000000,remaining=10000000,proposed=2000000,managers=['@chris:test'],calendar=dict(cadence='monthly',timeZone='America/Los_Angeles'))
            host=preview_host(a.binary,output,language,narrow,fixture)
            try:
                host.event('ux:native-panel')
                host.capture('overview')
                host.reveal('monthly')
                host.wait(lambda: host.row('monthly'),'native calendar')
                host.capture('calendar')
                # Preview drives the production control handler; it has no lease to publish.
                host.click('weekly');host.wait(lambda:'✓' in host.row('weekly')['t'],'weekly selected')
                host.reveal('zone');host.capture('weekly')
                host.click('zone');host.req('/k',c='A',cmd=1,wait=1);host.req('/t',t='Invalid/Zone',wait=1)
                host.capture('invalid-zone-before-check')
                (output/'invalid-zone-snap.json').write_text(json.dumps(host.snap(),ensure_ascii=False,indent=2))
                host.wait(lambda:not host.row('primary')['enabled'],'invalid IANA zone blocks publishing')
                host.capture('invalid-zone')
                host.click('zone');host.req('/k',c='A',cmd=1,wait=1);host.req('/t',t='Asia/Shanghai',wait=1)
                host.wait(lambda:host.row('primary')['enabled'],'valid zone enables publication')
                host.capture('shanghai')
                reports.append(dict(language=language,narrow=narrow,case='editable calendar',passed=True))
            finally:host.close()
            for case in ('bound calendar','blocked publication'):
                changes=dict(boundCalendar=fixture['calendar']) if case=='bound calendar' else dict(remaining=0,blockedReason='pool_limit_reached')
                detail=output/case.replace(' ','-')
                host=preview_host(a.binary,detail,language,narrow,dict(fixture,**changes))
                try:
                    host.event('ux:native-panel')
                    if case=='bound calendar':
                        host.scroll(250)
                        host.wait(lambda:any('America/Los_Angeles' in row.get('t','') for row in host.snap()),'bound owner calendar fact')
                        width=host.req('/s')['w'][0]['sz'][0]
                        notes=[row for row in host.snap() if 'America/Los_Angeles' in row.get('t','')]
                        assert all(row['r'][0]>=0 and row['r'][0]+row['r'][2]<=width for row in notes)
                        assert not any(row.get('i')=='monthly' and row['r'][2]>0 for row in host.snap())
                    else:
                        title='Cannot Publish' if language=='en' else '无法发布'
                        host.wait(lambda:host.row('title')['t']==title,'clear capacity refusal')
                        assert not host.row('primary')['enabled']
                    host.capture('result')
                    reports.append(dict(language=language,narrow=narrow,case=case,passed=True))
                finally:host.close()
    (a.output/'report.json').write_text(json.dumps(reports,indent=2));print(json.dumps(reports))
if __name__=='__main__':main()
