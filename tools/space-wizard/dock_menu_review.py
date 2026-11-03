#!/usr/bin/env python3
"""Headless production + menu → editor dock test with an isolated offline account."""
import argparse
import json
import os
from pathlib import Path
import time
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from native_review import ROOT
from member_invite_review import InviteReview

# Reuse the hidden Metal drawable retry for captures only; never retry clicks
# or failed state assertions.
class DockReview(InviteReview):
    pass


def run(binary, output, language):
    output = output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    profile = Path(tempfile.mkdtemp(prefix='profile-', dir=output))
    (profile/'.article-review-profile').touch()
    os.environ['HAGENCY_DOCK_OFFLINE_ACCOUNT'] = '1'
    hits = {}
    class Page(BaseHTTPRequestHandler):
        def do_GET(self):
            hits[self.path] = hits.get(self.path, 0) + 1
            body = ('<!doctype html><title>Workspace '+self.path+'</title><h1>Retained web tab '+self.path+'</h1><input value="Unsent web draft">').encode()
            self.send_response(200)
            self.send_header('Content-Type', 'text/html; charset=utf-8')
            self.send_header('Cache-Control', 'no-store')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        def log_message(self, *_):
            pass
    server = ThreadingHTTPServer(('127.0.0.1', 0), Page)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    origin = f'http://127.0.0.1:{server.server_port}'
    os.environ['HAGENCY_DOCK_URL'] = origin + '/one'
    app = DockReview(binary, output, (language, 1200, 850, False, False), profile=profile)
    def inspect():
        app.request('/event', data='dock:inspect', wait=1)
        time.sleep(.1)
        return json.loads((profile/'dock-inspection.json').read_text())
    def click_window(ident, window):
        button = next(w for w in app.request('/snap', w=window)['s'] if w.get('i') == ident)
        x,y,w,h = button['r']
        app.request('/click', w=window, x=x+w/2, y=y+h/2, wait=1)
        time.sleep(.2)
    passed = False
    try:
        app.request('/event', data='dock:light')
        time.sleep(.4)
        app.click('plus')
        for ident in ['add_room_or_space', 'add_article_editor', 'add_join_rooms']:
            x,y,w,h = app.widget(ident)['r']
            assert h >= 44 and x >= 0 and x+w <= 1200 and y+h <= 850
        app.capture('01-plus-menu')
        app.click('add_article_editor')
        app.click('article_continue')
        app.click('article_allow')
        assert len(app.request('/s')['w']) == 1
        app.checks.append('Production plus menu opens the editor in the existing window')
        app.click('article_new')
        title = '无窗口验收草稿' if language == 'zh-CN' else 'Headless review draft'
        app.fill('write_title_small', title)
        app.click('write_mode_source')
        source = '## Headless UX review\n\nThe same draft survives tab switches, theme changes and docking.\n'
        app.fill('write_source', source)
        before = {i:app.widget(i)['r'] for i in ['write_source','write_mode_source','write_publish']}
        time.sleep(1.2)
        assert before == {i:app.widget(i)['r'] for i in before}
        assert app.widget('draft')['val'] == 'Unsent chat draft'
        app.capture('02-docked-editor')
        app.checks.append('Native input and autosave preserve editor geometry and the unsent chat draft')
        app.click('read')
        app.capture('03-reader-tab')
        app.click('plus')
        app.click('add_article_editor')
        assert app.widget('write_source')['val'] == source
        assert app.widget('write_title_small')['val'] == title
        assert len([w for w in app.snap() if w.get('i')=='title' and w.get('t') in ('Article editor','文章编辑器')]) == 1
        app.checks.append('Reopening through the plus menu focuses one editor tab and retains the draft')
        app.request('/event', data='dock:dark')
        time.sleep(.3)
        assert app.widget('write_source')['val'] == source
        app.capture('04-dark-editor')
        app.checks.append('Theme change retains the active editor and draft')
        app.click('apps')
        app.click('web')
        app.request('/event', data='dock:review-web:' + origin + '/two')
        for _ in range(50):
            if hits.get('/one') and hits.get('/two'):
                break
            time.sleep(.1)
        assert hits.get('/one') == hits.get('/two') == 1, hits
        app.click('edit')
        docked = inspect()['docked']
        assert len(docked['tabs']) == 3
        assert len(docked['reader']['tabs']) == 2
        app.click('pop_out')
        windows = app.request('/s')['w']
        assert len(windows) == 2
        popped = next(w['i'] for w in windows if w['i'] != 0)
        nodes = app.request('/snap', w=popped)['s']
        moved = inspect()
        assert moved['docked']['tabs'] == [], 'Undock must move every workspace tab'
        assert moved['floating'] == docked, 'Tab order, identities, selection and browser session must all survive'
        assert next(w for w in nodes if w.get('i')=='write_source')['val'] == source
        app.capture('05-undocked', window=popped)
        # Main-window entry points must route into the existing floating group,
        # even when they target a tab that is currently in the background.
        app.click('web')
        selected = inspect()
        assert selected['docked']['tabs'] == []
        assert selected['floating']['tabs'] == docked['tabs']
        app.request('/click', w=popped, x=16, y=50, wait=1)
        app.capture('05a-floating-web', window=popped)
        app.request('/event', data='dock:review-web:' + origin + '/three')
        for _ in range(50):
            if hits.get('/three'): break
            time.sleep(.1)
        assert hits.get('/three') == 1, hits
        floating = inspect()['floating']
        assert len(floating['reader']['tabs']) == 4
        requests_before_docking = {path:hits.get(path, 0) for path in ['/one','/two','/three']}
        assert requests_before_docking == {'/one':2,'/two':1,'/three':1}, requests_before_docking
        click_window('dock_back', popped)
        time.sleep(.3)
        assert inspect()['docked'] == floating, 'Dock back returns every tab, including newly opened pages'
        app.click('edit')
        assert app.widget('write_source')['val'] == source
        assert len(app.request('/s')['w']) == 1
        app.capture('06-docked-back')
        app.checks.append('All mini-app/editor/reader tabs move together; background and newly opened web tabs follow the floating workspace; order, selection and draft survive')
        before_repeat = inspect()['docked']
        app.click('pop_out')
        second = next(w['i'] for w in app.request('/s')['w'] if w['i'] != 0)
        assert inspect()['floating'] == before_repeat
        click_window('dock_back', second)
        assert inspect()['docked'] == before_repeat
        assert {path:hits.get(path, 0) for path in requests_before_docking} == requests_before_docking, hits
        app.checks.append('Repeated dock/undock retains browser instances without reloading any of four uncached web tabs')
        library = json.loads(next(profile.glob('mini-apps/**/library-v2.json')).read_text())
        assert any(d['title']==title for d in library['documents'])
        app.checks.append('Draft is saved only in the marked offline profile')
        passed = True
        print(language, 'PASS', len(app.checks), flush=True)
    finally:
        app.close(passed)
        server.shutdown()
        server.server_close()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, default=ROOT/'target/debug/examples/miniapp_dock')
    parser.add_argument('--output', type=Path, default=ROOT/'.run/space-wizard-ios/menu-headless')
    args = parser.parse_args()
    for language in ['zh-CN','en']:
        run(args.binary, args.output/language, language)


if __name__ == '__main__':
    main()
