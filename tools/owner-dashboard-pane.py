#!/usr/bin/env python3
"""Capture an actually measured embedded dashboard pane in an owned fixture.

The host keeps its production dock minimum. --component explicitly constrains
the existing dashboard instance with Makepad's own tweaker for a narrower
component test; the report never calls that a production dock size.
"""
import argparse, hashlib, importlib.util, json, os, re, subprocess, time
from pathlib import Path

ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('owner_ax',ROOT/'tools/owner-dashboard-ax.py')
ax=importlib.util.module_from_spec(spec);spec.loader.exec_module(ax)

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--width',type=int,required=True)
    p.add_argument('--height',type=int,required=True)
    p.add_argument('--scale',type=float,default=1)
    p.add_argument('--component',action='store_true')
    args=p.parse_args();binary=args.binary.resolve();out=args.output.resolve();out.mkdir(parents=True,exist_ok=True)
    profile=out/'profile';profile.mkdir(exist_ok=True);(profile/'.article-review-profile').touch();(profile/'ui-language.json').write_text('"en"')
    dock_width=max(420,args.width);outer_width=dock_width+504;outer_height=args.height+235
    env=dict(os.environ,MAKEPAD_HIDE_WINDOWS='1',HAGENCY_DESKTOP_DATA_DIR=str(profile),
        HAGENCY_DOCK_OFFLINE_ACCOUNT='1',HAGENCY_OWNER_REVIEW_WIDTH=str(dock_width),
        HAGENCY_OWNER_REVIEW_SCALE=str(args.scale),HAGENCY_OWNER_REVIEW_HOST_SCALE='1',
        HAGENCY_DOCK_REVIEW_SIZE=f'{outer_width}x{outer_height}')
    env.pop('MAKEPAD_FOCUS',None)
    report={'binary':str(binary),'binarySha256':hashlib.sha256(binary.read_bytes()).hexdigest(),
        'requestedPane':[args.width,args.height],'dashboardScale':args.scale,
        'mode':'explicit component constraint inside production host' if args.component else 'production dock',
        'productionDockMinimum':420,'checks':[],'passed':False}
    process=journey=None;logfile=out/'fixture.log'
    def check(label,condition):
        report['checks'].append({'check':label,'passed':bool(condition)});assert condition,label
    try:
        with logfile.open('w') as log:
            process=subprocess.Popen([str(binary),'--remote'],cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT)
            deadline=time.monotonic()+30;match=None
            while time.monotonic()<deadline:
                match=re.search(r'listening on 127.0.0.1:(\d+) pid=(\d+)',logfile.read_text())
                if match:break
                assert process.poll() is None,'fixture exited'
                time.sleep(.1)
            assert match and int(match[2])==process.pid,'owned fixture endpoint missing'
            journey=ax.Journey('http://127.0.0.1:'+match[1],out)
            deadline=time.monotonic()+20
            while not journey.get('/s').get('w'):
                assert time.monotonic()<deadline,'window missing';time.sleep(.1)
            def settle():journey.get('/g');journey.get('/g')
            def rows():return [r for r in journey.get('/snap',w=0)['s'] if r['r'][2]>0 and r['r'][3]>0]
            def find(name):return next((r for r in rows() if r.get('i')==name),None)
            journey.scenario('running');settle()
            # Resize the actual NSWindow until the host pane has the requested
            # available size. Record every measured correction, not just env.
            report['resizeSteps']=[]
            for _ in range(4):
                host=find('owner_host');assert host,'owner host missing'
                report['resizeSteps'].append({'window':journey.get('/s')['w'][0]['sz'],'ownerHost':host['r']})
                dw=dock_width-host['r'][2];dh=args.height-host['r'][3]
                if abs(dw)<1 and abs(dh)<1:break
                outer_width+=round(dw);outer_height+=round(dh)
                journey.event(f'dock:window-size:{outer_width}x{outer_height}');journey.scenario('running');settle()
            host=find('owner_host');check('production host has the measured requested available pane',abs(host['r'][2]-dock_width)<1 and abs(host['r'][3]-args.height)<1)
            if args.component:
                edit=journey.get('/tweak/apply',p='owner_host.dashboard',splash=f'width: {args.width} height: {args.height}',wait=1)
                report['componentOverride']=edit
                journey.get('/tweak',on=0,wait=1);settle()
            dashboard=find('dashboard');assert dashboard,'dashboard instance missing'
            report['actualWindow']=journey.get('/s')['w'][0]
            report['actualHostBounds']=find('owner_host')['r'];report['actualDashboardBounds']=dashboard['r']
            check('actual dashboard dimensions match the request',abs(dashboard['r'][2]-args.width)<1 and abs(dashboard['r'][3]-args.height)<1)
            def shot(name):
                journey.get('/m',k='move',x=2,y=2,w=0,wait=1);time.sleep(.2);journey.grab(name)
                (out/(name+'-bounds.json')).write_text(json.dumps(rows(),ensure_ascii=False,indent=2)+'\n')
            def tab(label):
                if find('od_destination'):
                    journey.press('Dashboard page','page-open-'+label.lower())
                journey.press(label,'page-'+label.lower());settle()
            shot('overview')
            for page in ['Resources','Usage','Activity']:
                tab(page);shot(page.lower())
            tab('Overview');journey.press('Stop hosting','stop-open');settle();shot('stop-confirmation')
            cancel=find('od_stop_cancel');stop=find('od_stop_confirm')
            check('both confirmation actions fully visible',cancel and stop and min(cancel['r'][3],stop['r'][3])>=44)
            check('confirmation title and explanation visible',find('od_stop_title') and find('od_stop_body'))
            journey.native('final-native');journey.semantic('final-semantic')
            report['passed']=True
    except Exception as error:
        report['error']=str(error)
        if journey:
            try:journey.grab('failure')
            except Exception:pass
        raise
    finally:
        if process:
            if journey and process.poll() is None:
                try:journey.get('/quit')
                except Exception:pass
            try:process.wait(timeout=10)
            except subprocess.TimeoutExpired:process.terminate();process.wait(timeout=5)
        (out/'report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
    print(json.dumps(report,ensure_ascii=False))

if __name__=='__main__':main()
