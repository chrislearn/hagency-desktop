#!/usr/bin/env python3
"""Switch production Hosting languages through the UI and verify restart persistence."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time
from urllib.parse import urlencode
from urllib.request import urlopen
from urllib.error import HTTPError

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
binary, out = args.binary.resolve(), args.output.resolve()
if binary.suffix == '.app': binary = binary/'Contents/MacOS/hagency-owner-controller'
out.mkdir(parents=True, exist_ok=True)
domain = 'io.hagency.owner-controller.language-test.'+str(os.getpid())
process = None
checks = []
report = dict(passed=False, checks=checks, preferenceDomain=domain)


def check(name, condition):
    checks.append(dict(check=name, passed=bool(condition)))
    assert condition, name


def stop():
    global process
    if process and process.poll() is None:
        process.terminate()
        try: process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill(); process.wait(timeout=5)
    process = None


with tempfile.TemporaryDirectory(prefix='hosting-language-') as temp:
    home = Path(temp)
    env = dict(os.environ, MAKEPAD_HIDE_WINDOWS='1', MAKEPAD_REMOTE='127.0.0.1:0',
               HAGENCY_HOSTING_PREFERENCE_DOMAIN=domain)
    try:
        def start(name):
            global process, port
            log = out/(name+'.log')
            with log.open('w') as stream:
                process = subprocess.Popen([str(binary),'--daemon-home',str(home),'--setup'],
                    env=env, cwd=home, stdout=stream, stderr=stream)
            deadline = time.monotonic()+30
            while time.monotonic()<deadline:
                assert process.poll() is None, log.read_text(errors='replace')[-2500:]
                match = re.search(r'listening on 127\.0\.0\.1:(\d+)', log.read_text(errors='replace'))
                if match:
                    port = int(match[1])
                    frames = 0
                    while time.monotonic()<deadline and frames<5:
                        try:
                            request('g'); frames += 1
                        except HTTPError as error:
                            if error.code != 404: raise
                        time.sleep(.3)
                    assert frames==5, 'Controller did not draw its window'
                    return
                time.sleep(.15)
            raise AssertionError('No controller automation endpoint')

        def request(path, **params):
            with urlopen(f'http://127.0.0.1:{port}/'+path+'?'+urlencode(dict(w=0,wait=1,**params)),timeout=10) as response:
                return json.load(response)

        def nodes(): return request('snap').get('s',[])
        def find(ident):
            for _ in range(50):
                found = [n for n in nodes() if n.get('i')==ident and n.get('r') and n['r'][2]>0 and n['r'][3]>0]
                if found: return found[0]
                time.sleep(.1)
            raise AssertionError('Control not rendered: '+ident)
        def click(ident):
            x,y,w,h=find(ident)['r'];request('click',x=x+w/2,y=y+h/2)
        def key(code): request('k',c=code)
        def choose_language(label):
            click('hosting_language')
            # PopupMenuItem exposes its row ID, but not text, in Makepad's
            # remote snapshot. Click that rendered row and verify the chosen
            # label and the production screen below; do not inject an action.
            ident = {'System default': '-', 'English': '1', '简体中文': '2'}[label]
            for _ in range(30):
                request('g')
                items = [n for n in nodes() if n.get('ty')=='PopupMenuItem' and n.get('i')==ident
                         and n.get('r') and n['r'][2]>0 and n['r'][3]>0]
                if items: break
                time.sleep(.1)
            assert len(items)==1, (label,items)
            x,y,w,h=items[0]['r'];request('click',x=x+w/2,y=y+h/2)
            request('g');time.sleep(.4)
            assert find('hosting_language').get('selected')==label
        def saved():
            return subprocess.check_output(['defaults','read',domain,'HagencyLanguage'],text=True).strip()
        def shot(name):
            for _ in range(3): image=request('g');time.sleep(.3)
            shutil.copy2(image['png'],out/(name+'.png'))

        start('first')
        check('Language selector is visible before setup',bool(find('hosting_language')))
        choose_language('English')
        check('English is applied and saved',find('setup_title').get('t')=='Keep your Agents available' and saved()=='en')
        choose_language('简体中文')
        check('Chinese is applied and saved',find('setup_title').get('t')=='让 Agent 持续在线' and saved()=='zh-Hans')
        shot('chinese-setup')
        request('m',k='scroll',x=400,y=600,dy=500,precise=1)
        request('g');time.sleep(.3)
        click('setup_back')
        request('g')
        check('Dashboard switches to Chinese',any('托管' in n.get('t','') for n in nodes()))
        shot('chinese-dashboard')
        stop();start('restarted-chinese')
        check('Chinese survives restart',find('setup_title').get('t')=='让 Agent 持续在线')
        choose_language('English')
        time.sleep(2.5)
        check('English remains after system preference polling',find('setup_title').get('t')=='Keep your Agents available' and saved()=='en')
        shot('english-setup')
        stop();start('restarted-english')
        check('English survives restart',find('setup_title').get('t')=='Keep your Agents available')
        choose_language('System default')
        check('System default is saved',saved()=='system')
        report['passed']=True
        print(json.dumps(report,ensure_ascii=False))
    finally:
        if not report['passed'] and process and process.poll() is None:
            try:
                shot('failure')
                (out/'failure-nodes.json').write_text(json.dumps(nodes(),ensure_ascii=False,indent=2))
            except Exception:
                pass
        stop()
        (out/'report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
        subprocess.run(['defaults','delete',domain],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
