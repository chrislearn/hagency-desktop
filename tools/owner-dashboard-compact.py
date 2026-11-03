#!/usr/bin/env python3
"""Own-window large-text layout and banner-preserving scroll regression.

Runs only a synthetic standalone fixture. Retains screenshots, actual control
bounds, native semantic identity, handover state, binary hash and assertions.
"""
import argparse,hashlib,importlib.util,json,os,re,subprocess,time
from pathlib import Path

ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('owner_ax',ROOT/'tools/owner-dashboard-ax.py')
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--scale',type=float,default=2)
    parser.add_argument('--host-scale',type=float,default=1)
    parser.add_argument('--locale',choices=['en','zh'],default='en')
    parser.add_argument('--width',type=int,default=400)
    args=parser.parse_args();out=args.output.resolve();out.mkdir(parents=True,exist_ok=True)
    report={'binary':str(args.binary.resolve()),'binarySha256':hashlib.sha256(args.binary.read_bytes()).hexdigest(),
        'size':[args.width,540],'scale':args.scale,'hostScale':args.host_scale,'locale':args.locale,'checks':[]}
    env=dict(os.environ,MAKEPAD_HIDE_WINDOWS='1',HAGENCY_OWNER_REVIEW_HOST_SCALE=str(args.host_scale))
    env.pop('MAKEPAD_FOCUS',None)
    logfile=out/'fixture.log';process=None;journey=None
    def check(label,condition):
        report['checks'].append({'check':label,'passed':bool(condition)})
        assert condition,label
    try:
        with logfile.open('w') as log:
            process=subprocess.Popen([str(args.binary.resolve()),'--remote','--fixture=running',f'--size={args.width}x540',
                f'--text-scale={args.scale}',f'--locale={args.locale}','--appearance=light','--reduced-motion'],cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT)
            deadline=time.monotonic()+30;match=None
            while time.monotonic()<deadline:
                match=re.search(r'listening on 127.0.0.1:(\d+) pid=(\d+)',logfile.read_text())
                if match:break
                assert process.poll() is None,'fixture exited during startup'
                time.sleep(.1)
            assert match and int(match[2])==process.pid,'no owned fixture endpoint'
            journey=module.Journey('http://127.0.0.1:'+match[1],out,False)
            deadline=time.monotonic()+20
            while not journey.get('/s').get('w'):
                assert time.monotonic()<deadline,'no fixture window'
                time.sleep(.1)
            def settle():
                journey.get('/g');journey.get('/g')
            def rows():return journey.get('/snap',w=0)['s']
            def find(name):return next((r for r in rows() if r.get('i')==name and r['r'][2]>0 and r['r'][3]>0),None)
            def shot(name):
                journey.grab(name)
                (out/(name+'-bounds.json')).write_text(json.dumps(rows(),ensure_ascii=False,indent=2))
            def key(k,**kw):journey.get('/k',c=k,wait=1,**kw);settle()
            def scroll(delta):
                x,y,w,h=find('od_content')['r'];journey.get('/m',k='scroll',x=x+w*.7,y=y+h*.6,dy=delta,precise=1,w=0,wait=1);settle()
            def click(name):
                for _ in range(60):
                    target=find(name)
                    if target and target['r'][3]>=40:break
                    scroll(180)
                assert target and target['r'][3]>=40,'unreachable '+name
                x,y,w,h=target['r'];journey.get('/click',x=x+w/2,y=y+h/2,w=0,wait=1);settle()
            def state(name):return journey.capture('dump','od-state ',name)
            settle()
            key('3',logo=1);shot('usage-initial')
            header=find('od_header')['r'];total=find('total')
            check('compact header leaves most of the window for content',header[3] < (540-32)/2)
            check('initial Usage contains the actual total',total is not None and total.get('t') not in ('','—') and total['r'][3]>=30)
            before=journey.semantic('before-owner-expand')
            click('od_owner_menu');shot('owner-expanded')
            check('owner disclosure exposes Refresh and host navigation',find('od_refresh') is not None and find('od_open_other') is not None)
            journey.event('refresh-step:1');settle()
            check('refresh retains expanded owner controls',find('od_refresh') is not None)
            click('od_owner_menu');after=journey.semantic('after-owner-collapse')
            def by_name(tree,name):
                return [(uid,node) for win in tree['windows'] for uid,node in win['tree']['nodes'] if node.get('properties',{}).get('label')==name]
            page_label='控制台页面' if args.locale=='zh' else 'Dashboard page'
            before_page=by_name(before,page_label);after_page=by_name(after,page_label)
            check('disclosure preserves native page control identity',bool(before_page) and before_page==after_page)
            key('2',logo=1);shot('resources')
            name=find('name');line=find('line');tokens=find('tokens');details=find('od_resource_open')
            check('resource metadata has full readable column',line is not None and line['r'][2] >= args.width-70)
            check('tokens and Details sit below metadata',tokens['r'][1]>line['r'][1] and details['r'][1]>line['r'][1])
            click('od_resource_open');shot('resource-detail');key('Escape')
            key('1',logo=1);click('od_stop');shot('stop-confirmation')
            title=find('od_stop_title');body=find('od_stop_body');cancel=find('od_stop_cancel');stop=find('od_stop_confirm')
            check('Stop title and explanation remain visible',title is not None and body is not None and body['r'][3]>=45)
            check('Cancel and Stop are fully visible together',cancel is not None and stop is not None and min(cancel['r'][3],stop['r'][3])>=44 and abs(cancel['r'][1]-stop['r'][1])<1)
            check('confirmation focuses Cancel',state('stop-state')['focus']==(['取消'] if args.locale=='zh' else ['Cancel']))
            click('od_stop_cancel');check('Cancel keeps hosting',not state('cancel-state')['confirmStop'])
            journey.scenario('uncertain');settle();shot('uncertain-initial')
            check('uncertain command context and status action visible together',find('od_command_status') is not None and find('od_check_command') is not None)
            journey.scenario('large');key('2',logo=1)
            settle();scroll(320)
            resource_ids={'name','line','budget','tokens','od_resource_open'}
            reading_before=[(r.get('i'),r.get('t'),r['r']) for r in rows() if r.get('i') in resource_ids]
            journey.event('notice:Fixture handover notice');journey.event('inventory:partial');settle()
            reading_after=[(r.get('i'),r.get('t'),r['r']) for r in rows() if r.get('i') in resource_ids]
            check('new notices preserve the current reading position',bool(reading_before) and reading_before==reading_after)
            saved=state('scroll-before-handover')['view'];shot('scroll-before-handover')
            check('banner-independent actual scroll matches wheel delta',abs(saved['scrollY']-320)<2)
            visible_before=[(r.get('i'),r.get('t'),r['r']) for r in rows() if r.get('t')]
            reset=dict(saved,scrollY=0);journey.event('restore-view:'+json.dumps(reset));settle()
            journey.event('restore-view:'+json.dumps(saved));settle();shot('scroll-after-handover')
            restored=state('scroll-after-handover')['view']
            visible_after=[(r.get('i'),r.get('t'),r['r']) for r in rows() if r.get('t')]
            check('handover restores true scroll with notice and inventory banner',restored['scrollY']==saved['scrollY'] and visible_before==visible_after)
            journey.scenario('long-names');key('3',logo=1)
            click('od_usage_filters_toggle');click('od_filter_account');click('picker_search')
            journey.get('/k',t='really-long',wait=1);settle();shot('picker-search')
            search_state=state('picker-search-state')
            key('ArrowDown');shot('picker-keyboard-choice')
            choice_state=state('picker-choice-state');semantic=journey.semantic('picker-choice-semantic')
            focused=[node for win in semantic['windows'] for uid,node in win['tree']['nodes'] if uid==win['tree']['focus']]
            bounds=focused[0].get('properties',{}).get('bounds',{}) if focused else {}
            content=find('od_content')['r']
            check('ArrowDown gives the matching full-name choice actual keyboard focus',
                len(choice_state['focus'])==1 and 'really-long' in choice_state['focus'][0]
                and focused and focused[0].get('role')=='radioButton')
            check('keyboard choice is revealed inside the content viewport',
                bool(bounds) and bounds['y0']>=content[1]-1 and (
                    bounds['y1']<=content[1]+content[3]+1 or (
                        bounds['y1']-bounds['y0']>content[3]-10 and abs(bounds['y0']-content[1]-5)<1)))
            focus_id=next(win['tree']['focus'] for win in semantic['windows'] if any(uid==win['tree']['focus'] and node.get('role')=='radioButton' for uid,node in win['tree']['nodes']))
            journey.native('picker-repeat-native-focus','focus:'+str(focus_id));settle()
            repeated=state('picker-repeat-focus-state')
            check('repeating native focus preserves the revealed name position',
                abs(repeated['view']['scrollY']-choice_state['view']['scrollY'])<1 and repeated['focus']==choice_state['focus'])
            key('Return');selected_state=state('picker-selection-state')
            check('Return selects the keyboard-focused account',selected_state['view']['accountFilter']=='acct_codex')
            report['pickerKeyboard']={'beforeScroll':search_state['view']['scrollY'],'afterScroll':choice_state['view']['scrollY'],'bounds':bounds,'content':content}
            report['headerBounds']=header;report['usageTotalBounds']=total['r'];report['confirmationBounds']={k:v['r'] for k,v in [('title',title),('body',body),('cancel',cancel),('stop',stop)]}
            report['passed']=True
    except Exception as error:
        report['passed']=False;report['error']=str(error)
        if journey:
            try:journey.grab('failure')
            except Exception:pass
        raise
    finally:
        if process:
            if journey and process.poll() is None:
                try:journey.get('/quit')
                except Exception:pass
            try:process.wait(timeout=5)
            except subprocess.TimeoutExpired:process.terminate();process.wait(timeout=5)
        (out/'report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
    print(json.dumps(report,ensure_ascii=False))

if __name__=='__main__':main()
