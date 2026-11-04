#!/usr/bin/env python3
"""Exercise LaunchServices/Dock identity and controller-independent daemon life.

Uses a fresh, unenrolled daemon home. Never installs a service, signs in, sends
inference, or touches a running Hagency profile. This does not prove encrypted
Agent execution; that requires the enrolled runtime integration tests.
"""
import argparse
import hashlib
from http.client import RemoteDisconnected
import json
import os
from pathlib import Path
import plistlib
import shutil
import signal
import socket
import subprocess
import tempfile
import time
from urllib.request import urlopen
from urllib.error import URLError

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--bundle', type=Path, required=True)
p.add_argument('--output', type=Path, required=True)
args = p.parse_args()
bundle, out = args.bundle.resolve(), args.output.resolve()
out.mkdir(parents=True, exist_ok=True)
helper = bundle/'Contents/Helpers/hagency'
report = dict(bundle=str(bundle), checks=[], passed=False,
              daemonSha256=hashlib.sha256(helper.read_bytes()).hexdigest(),
              controllerSha256=hashlib.sha256((bundle/'Contents/MacOS/hagency-owner-controller').read_bytes()).hexdigest())


def check(name, condition, **evidence):
    report['checks'].append(dict(check=name, passed=bool(condition), **evidence))
    assert condition, name


def running():
    script = '''function run(argv) {
      ObjC.import('AppKit'); var result=[];
      var apps=$.NSRunningApplication.runningApplicationsWithBundleIdentifier('io.hagency.owner-controller');
      for(var i=0;i<apps.count;i++) {var a=apps.objectAtIndex(i);
        if(ObjC.unwrap(a.bundleURL.path)===argv[0]) result.push({pid:Number(a.processIdentifier),policy:Number(a.activationPolicy)});
      } return JSON.stringify(result);
    }'''
    return json.loads(subprocess.check_output(['/usr/bin/osascript','-l','JavaScript','-e',script,str(bundle)], text=True))


def req(port, path):
    return json.load(urlopen(f'http://127.0.0.1:{port}'+path, timeout=10))


def wait_for(predicate, seconds=15):
    deadline = time.monotonic()+seconds
    while time.monotonic() < deadline:
        try:
            value = predicate()
            if value:
                return value
        except (OSError, ValueError, subprocess.CalledProcessError):
            pass
        time.sleep(.15)
    raise AssertionError('timed out waiting for isolated app state')


def launch(home, name):
    with socket.socket() as sock:
        sock.bind(('127.0.0.1',0))
        port = sock.getsockname()[1]
    subprocess.run(['/usr/bin/open','-n','-g','-j',
                    '--env','MAKEPAD_HIDE_WINDOWS=1','--env','MAKEPAD_NO_FOCUS=1',
                    '--env',f'MAKEPAD_REMOTE={port}',
                    '-o',str(out/(name+'.log')),'--stderr',str(out/(name+'-stderr.log')),
                    str(bundle),'--args','--daemon-home='+str(home)], check=True)
    wait_for(lambda: req(port,'/s').get('w'))
    req(port,'/g?w=0&scale=1')
    capture=req(port,'/g?w=0&scale=1')
    shutil.copyfile(capture['png'],out/(name+'.png'))
    return port


daemon=None
ports=[]
try:
    subprocess.run(['/usr/bin/codesign','--verify','--deep','--strict',str(bundle)],check=True)
    info=plistlib.loads((bundle/'Contents/Info.plist').read_bytes())
    check('native application has its own regular Dock identity',
          info['CFBundleIdentifier']=='io.hagency.owner-controller' and not info.get('LSUIElement',False))
    check('this exact bundle is not already running', not running())
    with tempfile.TemporaryDirectory(prefix='odock-',dir='/tmp') as folder:
        home=Path(folder)/'daemon'
        with (out/'daemon.log').open('w') as log:
            daemon=subprocess.Popen([str(helper),'daemon','run','--home',str(home)],stdout=log,stderr=subprocess.STDOUT)
            def status():
                result=subprocess.run([str(helper),'daemon','status','--home',str(home)],text=True,capture_output=True,timeout=10)
                return result.stdout if result.returncode==0 else None
            before=wait_for(status)
            (out/'daemon-before.txt').write_text(before)
            first=launch(home,'first-open');ports.append(first)
            apps=running()
            check('LaunchServices creates one regular Dock application',len(apps)==1 and apps[0]['policy']==0, apps=apps)
            req(first,'/close?w=0')
            wait_for(lambda: not running())
            check('closing the last controller window leaves daemon alive',daemon.poll() is None and bool(status()))
            second=launch(home,'reopen');ports.append(second)
            check('Dock relaunch creates a fresh controller process',len(running())==1 and running()[0]['pid']!=apps[0]['pid'])
            try:
                req(second,'/quit')
            except (RemoteDisconnected, URLError, ConnectionResetError):
                # Quit can stop the HTTP worker before it sends a response.
                # The process disappearance and live daemon are checked below;
                # a missing response alone is neither success nor failure.
                pass
            wait_for(lambda: not running())
            after=status()
            check('quitting reopened controller leaves the same daemon alive',daemon.poll() is None and bool(after))
            (out/'daemon-after.txt').write_text(after)
            daemon.send_signal(signal.SIGINT)
            daemon.wait(timeout=15)
            check('isolated daemon shuts down cleanly',daemon.returncode==0)
    report['passed']=True
finally:
    for port in ports:
        try:req(port,'/quit')
        except Exception:pass
    if daemon and daemon.poll() is None:
        daemon.send_signal(signal.SIGINT)
        try:daemon.wait(timeout=15)
        except subprocess.TimeoutExpired:daemon.terminate();daemon.wait(timeout=5)
    (out/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(dict(passed=report['passed'],checks=len(report['checks']),output=str(out))))
