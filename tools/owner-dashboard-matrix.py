#!/usr/bin/env python3
"""Hidden-window, isolated dashboard visual/retention matrix for either host.

Evidence is bound to the supplied executable; this is automated evidence, not
a visual quality score or a manual VoiceOver acceptance record.
"""
import argparse
import hashlib
import json
import os
import platform
from pathlib import Path
import shutil
import socket
import subprocess
import time
from urllib.parse import urlencode
from urllib.request import urlopen

ROOT = Path(__file__).resolve().parents[1]
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--binary', type=Path, required=True)
p.add_argument('--output', type=Path, required=True)
p.add_argument('--embedded', action='store_true')
p.add_argument('--scale', type=float, default=1, choices=[1, 1.25, 1.5, 2])
p.add_argument('--host-scale', type=float, default=1)
p.add_argument('--zoom', type=float, default=1)
p.add_argument('--width', type=int, default=400)
p.add_argument('--height', type=int, default=540)
args = p.parse_args()
out = args.output.resolve()
out.mkdir(parents=True, exist_ok=True)
profile = out / 'profile'
profile.mkdir(exist_ok=True)
(profile / '.article-review-profile').touch()
(profile / 'ui-language.json').write_text(json.dumps('en'))
sock = socket.socket()
sock.bind(('127.0.0.1', 0))
port = sock.getsockname()[1]
sock.close()
env = dict(os.environ, MAKEPAD_REMOTE=str(port), MAKEPAD_HIDE_WINDOWS='1',
           MAKEPAD_NO_FOCUS='1', HAGENCY_OWNER_DAEMON_HOME=str(out/'unused-daemon'),
           HAGENCY_DESKTOP_DATA_DIR=str(profile), HAGENCY_DOCK_OFFLINE_ACCOUNT='1',
           HAGENCY_OWNER_REVIEW_WIDTH=str(args.width), HAGENCY_OWNER_REVIEW_SCALE=str(args.scale),
           HAGENCY_OWNER_REVIEW_HOST_SCALE=str(args.host_scale))
env.pop('MAKEPAD_FOCUS', None)
command = [str(args.binary.resolve())]
if not args.embedded:
    command += ['--fixture=running', '--appearance=light', '--locale=en',
                f'--size={args.width}x{args.height}', '--text-scale='+str(args.scale)]
report = dict(binary=command[0], binarySha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
              host='embedded' if args.embedded else 'standalone', scale=args.scale,
              hostScale=args.host_scale, uiZoom=args.zoom,
              requestedSize=[args.width, args.height],
              os=platform.platform(), machine=platform.machine(), logicalCpus=os.cpu_count(), loadAtStart=os.getloadavg(),
              checks=[], passed=False)


def req(path, **params):
    return json.load(urlopen(f'http://127.0.0.1:{port}'+path+
                            ('?'+urlencode(params) if params else ''), timeout=20))


def event(command):
    req('/event', data=command, wait=1)
    req('/g', w=0, scale=1)


def scenario(name):
    event(('dock:owner:' if args.embedded else 'scenario:')+name)


def rows():
    return [r for r in req('/snap', w=0)['s'] if r['r'][2] > 0 and r['r'][3] > 0]


def find(ident):
    # Repeated list controls share an ID. Prefer a fully visible row over the
    # clipped tail of the preceding row at the viewport edge.
    return max((r for r in rows() if r.get('i') == ident),
               key=lambda row: row['r'][3], default=None)


def check(name, passed, **evidence):
    report['checks'].append(dict(check=name, passed=bool(passed), **evidence))
    assert passed, name


def shot(name):
    # Synthetic clicks do not synthesize pointer movement between controls.
    # Settle hover/pressed animations before judging persistent selection.
    req('/m', k='move', x=2, y=2, w=0, wait=1)
    time.sleep(.2)
    req('/g', w=0, scale=1)
    capture = req('/g', w=0, scale=1)
    shutil.copyfile(capture['png'], out/(name+'.png'))
    (out/(name+'.json')).write_text(json.dumps(rows(), ensure_ascii=False, indent=2))


def click(ident):
    target = find(ident)
    assert target, 'missing visible control '+ident
    x, y, w, h = target['r']
    req('/click', x=x+w/2, y=y+h/2, w=0, wait=1)


def scroll(dy):
    x, y, w, h = find('od_content')['r']
    req('/m', k='scroll', x=x+w*.8, y=y+h*.7, dy=dy, precise=1, w=0, wait=1)
    req('/g', w=0, scale=1)


def reveal(ident, minheight=35):
    for _ in range(80):
        target = find(ident)
        if target and target['r'][3] >= minheight:
            return target
        scroll(200)
    raise AssertionError('control unreachable: '+ident)


def tab(index):
    if find('od_destination'):
        click('od_destination')
        req('/k', c='Escape', wait=1)
        for _ in range(3):
            req('/k', c='ArrowUp', wait=1)
        for _ in range(index):
            req('/k', c='ArrowDown', wait=1)
    else:
        click(['od_tab_overview', 'od_tab_resources', 'od_tab_usage', 'od_tab_activity'][index])
    req('/g', w=0, scale=1)


def open_filters():
    # Scenario replacement may retain this owner's filter presentation. Read
    # its actual state rather than blindly toggling an already-open card.
    scroll(-20000)
    target = reveal('od_usage_filters_toggle')
    if not target.get('t', '').startswith('Hide filters'):
        click('od_usage_filters_toggle')
    req('/g', w=0, scale=1)
    req('/g', w=0, scale=1)


process = None
with (out/'app.log').open('w') as logfile:
    try:
        process = subprocess.Popen(command, cwd=ROOT, env=env, stdout=logfile, stderr=subprocess.STDOUT)
        for _ in range(200):
            if process.poll() is not None:
                raise RuntimeError('fixture exited')
            try:
                if req('/s')['w']:
                    break
            except OSError:
                pass
            time.sleep(.1)
        scenario('running')
        if args.zoom != 1:
            event(('dock:owner-zoom:' if args.embedded else 'zoom:')+str(args.zoom))
        report['measuredDashboardBounds'] = next((r['r'] for r in rows()
            if r.get('ty') == 'OwnerDashboard'), None)
        report['measuredWindows'] = req('/s').get('w')
        for locale in ['en', 'zh']:
            event(('dock:owner-locale:' if args.embedded else 'locale:')+locale)
            for appearance in ['light', 'dark']:
                event(('dock:' if args.embedded else 'appearance:')+appearance)
                for index, name in enumerate(['overview', 'resources', 'usage', 'activity']):
                    tab(index)
                    shot(f'{locale}-{appearance}-{name}')
                check(f'{locale}/{appearance}: all four destinations reachable', find('od_activity_page') is not None)
        event(('dock:owner-locale:' if args.embedded else 'locale:')+'en')
        event(('dock:' if args.embedded else 'appearance:')+'light')
        scenario('running')
        tab(2)
        open_filters()
        reveal('od_date_start')
        click('od_date_start')
        req('/k', c='a', logo=1, wait=1)
        req('/k', t='2026-10-14', wait=1)
        req('/k', c='ArrowLeft', shift=1, wait=1)
        before = {r['i']: r['r'] for r in rows() if r.get('i') in
                  ['od_content', 'od_date_start', 'od_date_end', 'od_destination', 'od_tab_usage']}
        for step in range(10):
            event(('dock:owner-refresh:' if args.embedded else 'refresh-step:')+str(step+1))
        after = {r['i']: r['r'] for r in rows() if r.get('i') in before}
        check('ten refreshes retain scrolled filter geometry', bool(before) and all(
            key in after and max(abs(a-b) for a,b in zip(bounds, after[key])) <= 1
            for key,bounds in before.items()), before=before, after=after)
        # A selected final character must still be selected and focused after
        # refresh. Normal text input replaces it, preserving the other draft.
        req('/k', t='5', wait=1)
        value = find('od_date_start')
        check('refresh preserves draft, focus and text selection', value is not None and value.get('t') == '2026-10-15', value=value)
        shot('refresh-retention')
        scroll(-20000)
        tab(2)
        latencies = []
        for _ in range(20):
            target = reveal('od_usage_filters_toggle')
            content = find('od_content')['r']
            if target['r'][1] > content[1]+content[3]*.35:
                scroll(target['r'][1]-content[1]-16)
                target = find('od_usage_filters_toggle')
            x, y, w, h = target['r']
            was_open = target.get('t', '').startswith('Hide filters')
            start = time.perf_counter()
            req('/click', x=x+w/2, y=y+h/2, w=0, wait=1)
            # Include instrumented UI acknowledgement, not only queue admission.
            deadline = start + .1
            while True:
                current = req('/snap', w=0)['s']
                toggle = next((r for r in current if r.get('i') == 'od_usage_filters_toggle'), None)
                is_open = toggle is not None and toggle.get('t', '').startswith('Hide filters')
                if is_open != was_open or time.perf_counter() >= deadline:
                    break
                req('/g', w=0, scale=1)
            latencies.append((time.perf_counter()-start)*1000)
            assert is_open != was_open, 'filter toggle did not produce an acknowledged UI change'
            req('/g', w=0, scale=1)
        p95 = sorted(latencies)[18]
        check('local filter input acknowledgement p95 <=100ms', p95 <= 100, p95_ms=p95, samples_ms=latencies)
        for name in ['daemon-absent', 'auth-expired', 'incompatible', 'starting', 'paused',
                     'draining', 'stopped', 'offline', 'empty', 'large', 'long-names',
                     'shared-account', 'unknown', 'stale', 'rejected', 'uncertain']:
            scenario(name)
            shot('state-'+name)
        scenario('shared-account')
        tab(0)
        plans = reveal('od_plans_header', 1)
        scroll(plans['r'][1]-find('od_content')['r'][1]-8)
        shot('provider-plans-shared-account')
        scroll(220)
        shot('provider-plans-shared-account-limits')
        for name in ['large', 'long-names', 'shared-account']:
            scenario(name)
            tab(1)
            shot(name+'-resources')
            if name == 'large':
                reveal('next')
                click('next')
                shot('large-resources-page2')
            reveal('od_resource_open')
            click('od_resource_open')
            shot(name+'-resource-detail')
            tab(2)
            open_filters()
            reveal('od_filter_resource')
            click('od_filter_resource')
            shot(name+'-resource-picker-open')
            req('/k', c='Escape', wait=1)
        scenario('large')
        tab(2)
        open_filters()
        reveal('od_filter_resource')
        click('od_filter_resource')
        reveal('picker_next')
        click('picker_next')
        choices = [r.get('t') for r in rows() if r.get('i') == 'picker_choice']
        check('resource picker reaches its second bounded page',
              any(label in choices for label in ['Pool Agent 004', 'Pool Agent 005', 'Pool Agent 006'])
              and 'All resources' not in choices, choices=choices)
        # Paging moves focus to a visible choice; return to the search field
        # using the parent scroll view, then exercise the ordinary text path.
        scroll(-20000)
        reveal('picker_search')
        click('picker_search')
        req('/k', t='059', wait=1)
        shot('resource-picker-search')
        req('/k', c='Return', wait=1)
        req('/g', w=0, scale=1)
        selected = next((r for r in rows() if r.get('i') == 'picker_toggle'
                         and 'Pool Agent 059' in r.get('t', '')), None)
        check('name search and Return select the matching resource',
              selected is not None and 'Pool Agent 059' in selected.get('t', ''), selected=selected)
        click('od_filter_resource')
        reveal('picker_search')
        query_before = find('picker_search')
        event(('dock:owner-refresh:' if args.embedded else 'refresh-step:')+'1')
        query_after = find('picker_search')
        check('same-owner refresh retains picker search draft',
              query_before is not None and query_after is not None
              and query_before.get('t') == query_after.get('t') == '059')
        req('/k', c='Escape', wait=1)
        shot('resource-picker-unavailable-selection')
        scenario('hosting-history')
        event(('dock:owner-locale:' if args.embedded else 'locale:')+'zh')
        tab(3)
        shot('zh-hosting-history')
        report['passed'] = True
    except Exception as error:
        report['error'] = str(error)
        try:
            shot('failure')
        except Exception:
            pass
        raise
    finally:
        (out/'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2)+'\n')
        if process and process.poll() is None:
            try:
                req('/quit')
            except Exception:
                pass
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.terminate()
                process.wait(timeout=5)
        print(json.dumps(dict(passed=report['passed'], output=str(out), checks=len(report['checks']))))
