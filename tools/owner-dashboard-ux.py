#!/usr/bin/env python3
"""Reproducible hidden-window UX journey; touches only its own fixture process."""
import argparse,hashlib,json,os,socket,subprocess,sys,time,urllib.request,urllib.parse
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser()
parser.add_argument('--binary',type=Path,required=True)
parser.add_argument('--output',type=Path,required=True)
parser.add_argument('--locale',default='en',choices=['en','zh'])
parser.add_argument('--appearance',default='light',choices=['light','dark'])
parser.add_argument('--scale',default='2',choices=['1','1.25','1.5','2'])
args=parser.parse_args()
OUT=args.output.resolve();OUT.mkdir(parents=True,exist_ok=True)
metadata={'binary':str(args.binary.resolve()),'binarySha256':hashlib.sha256(args.binary.read_bytes()).hexdigest(),'locale':args.locale,'appearance':args.appearance,'scale':args.scale,'host':'standalone','size':[400,540]}
(OUT/'metadata.json').write_text(json.dumps(metadata,indent=2))
s=socket.socket();s.bind(('127.0.0.1',0));port=s.getsockname()[1];s.close()
env=dict(os.environ,MAKEPAD_REMOTE=str(port),MAKEPAD_HIDE_WINDOWS='1',MAKEPAD_NO_FOCUS='1',HAGENCY_OWNER_DAEMON_HOME=str(OUT/'unused'))
log=(OUT/'app.log').open('w')
p=subprocess.Popen([str(args.binary.resolve()),'--fixture=running','--setup','--locale='+args.locale,'--appearance='+args.appearance,'--size=400x540'],cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT)
def req(path,**args):
 return json.load(urllib.request.urlopen('http://127.0.0.1:'+str(port)+path+('?' + urllib.parse.urlencode(args) if args else ''),timeout=10))
def event(cmd):req('/event',data=cmd);time.sleep(.1)
def state():
 event('dump')
 lines=req('/log',n=100)['l']
 return json.loads([l.split('od-state ',1)[1] for l in lines if 'od-state ' in l][-1])
def key(code,**extra):req('/k',c=code,wait=1,**extra);time.sleep(.08)
def text(t):req('/k',t=t,wait=1);time.sleep(.08)
def rows():return req('/snap',w=0)['s']
checks=[]
def check(name,condition):
 checks.append({'check':name,'passed':bool(condition)})
 (OUT/'report.json').write_text(json.dumps(checks,indent=2))
 assert condition,name
try:
 for _ in range(100):
  if p.poll() is not None:raise RuntimeError('fixture exited')
  try:
   if req('/s')['w']:break
  except OSError:pass
  time.sleep(.1)
 time.sleep(.7)
 event('scenario:running')
 # Leave setup with a real click, then use the window's macOS shortcuts.
 def find(ident):
  return next((r for r in rows() if r.get('i')==ident and r['r'][2]>0 and r['r'][3]>0),None)
 def scroll(dy):
  target=find('od_content') or find('setup_page')
  assert target, 'no scroll area'
  x,y,w,h=target['r'];req('/m',k='scroll',x=x+w*.7,y=y+h*.6,dy=dy,precise=1,w=0,wait=1);time.sleep(.2)
 def reveal(ident,minheight=35):
  for _ in range(100):
   target=find(ident)
   if target and target['r'][3]>=minheight:return target
   scroll(250)
  raise AssertionError('not reachable: '+ident)
 def click(ident):
  if ident in ['od_refresh','od_open_other'] and not find(ident) and find('od_owner_menu'):
   click('od_owner_menu')
  r=reveal(ident)['r'];req('/click',x=r[0]+r[2]/2,y=r[1]+r[3]/2,w=0,wait=1);time.sleep(.15)
 def shot(name):
  import shutil
  (OUT/(name+'.json')).write_text(json.dumps(rows(),ensure_ascii=False,indent=2))
  req('/g',w=0,scale=1);result=req('/g',w=0,scale=1);shutil.copyfile(result['png'],OUT/(name+'.png'))
 click('setup_back')
 event('text-scale:'+args.scale);time.sleep(.3)
 if find('od_destination'):
  click('od_destination');shot('destination-open')
  key('Escape');shot('destination-dismissed')
  key('ArrowDown');check('large-text dropdown selects Resources',state()['view']['tab']=='resources')
 else:key('2',logo=1)
 shot('resources')
 click('od_resource_open');check('resource opens its own detail',state()['view']['selectedResource'] is not None);shot('resource-detail')
 click('od_detail_back');check('detail Back preserves Resources',state()['view']['selectedResource'] is None)
 key('3',logo=1);check('Cmd-3 opens Usage',state()['view']['tab']=='usage');shot('usage')
 click('od_usage_filters_toggle');shot('filters')
 click('od_apply_dates');reveal('od_date_error',1);check('invalid custom dates are explained',find('od_date_error') is not None);shot('date-error')
 # Enter dates using the normal text input path and commit with the button.
 scroll(-20000)
 click('od_date_start');key('a',logo=1);text('2026-10-01')
 click('od_date_end');key('a',logo=1);text('2026-10-31')
 click('od_apply_dates');check('custom range selected',state()['view']['period']['kind']=='range');shot('custom-range')
 key('4',logo=1);check('Cmd-4 opens Activity',state()['view']['tab']=='activity');shot('activity')
 key('1',logo=1);check('Cmd-1 returns Overview',state()['view']['tab']=='overview')
 click('od_stop');reveal('od_stop_cancel');check('Stop requires confirmation',state()['confirmStop']);shot('stop-confirm-visible')
 click('od_stop_cancel');check('Cancel keeps hosting',not state()['confirmStop'])
 event('scenario:stopped');click('od_resume');check('Stopped can resume without setup',state()['command']['kind']=='Resume');shot('resume-requested')
 event('scenario:uncertain');reveal('od_check_command');shot('uncertain-visible')
 event('scenario:rejected');reveal('od_retry_command');shot('rejected-visible')
 event('scenario:running');event('setup');click('setup_enable');time.sleep(.3)
 check('invalid input moves focus to server',state()['focusId']=='setup_server')
 shot('setup-error-focused')
 print(json.dumps({'passed':True,'checks':len(checks),'output':str(OUT)}))
finally:
 error=sys.exc_info()[1]
 (OUT/'report.json').write_text(json.dumps({'passed':error is None,'error':str(error) if error else None,'checks':checks,**metadata},indent=2))
 if error:
  try:shot('failure')
  except Exception:pass
 try:req('/quit')
 except Exception:pass
 try:p.wait(timeout=5)
 except subprocess.TimeoutExpired:p.terminate();p.wait(timeout=5)
 log.close()
