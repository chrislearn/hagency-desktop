#!/usr/bin/env python3
"""Headless production + menu → editor dock test with an isolated offline account."""
import argparse
import json
import os
from pathlib import Path
import time
import tempfile
from native_review import Review, ROOT


def run(binary, output, language):
    output = output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    profile = Path(tempfile.mkdtemp(prefix='profile-', dir=output))
    (profile/'.article-review-profile').touch()
    os.environ['HAGENCY_DOCK_OFFLINE_ACCOUNT'] = '1'
    app = Review(binary, output, (language, 1200, 850, False, False), profile=profile)
    passed = False
    try:
        app.request('/event', data='dock:light')
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
        app.click('pop_out')
        windows = app.request('/s')['w']
        assert len(windows) == 2
        popped = next(w['i'] for w in windows if w['i'] != 0)
        nodes = app.request('/snap', w=popped)['s']
        assert next(w for w in nodes if w.get('i')=='write_source')['val'] == source
        app.capture('05-undocked', window=popped)
        button = next(w for w in nodes if w.get('i')=='dock_back')
        x,y,w,h = button['r']
        app.request('/click', w=popped, x=x+w/2, y=y+h/2, wait=1)
        time.sleep(.3)
        assert app.widget('write_source')['val'] == source
        assert len(app.request('/s')['w']) == 1
        app.capture('06-docked-back')
        app.checks.append('Undock and dock back retain the same editor draft')
        library = json.loads(next(profile.glob('mini-apps/**/library-v2.json')).read_text())
        assert any(d['title']==title for d in library['documents'])
        app.checks.append('Draft is saved only in the marked offline profile')
        passed = True
        print(language, 'PASS', len(app.checks), flush=True)
    finally:
        app.close(passed)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, default=ROOT/'target/debug/examples/miniapp_dock')
    parser.add_argument('--output', type=Path, default=ROOT/'.run/space-wizard-ios/menu-headless')
    args = parser.parse_args()
    for language in ['zh-CN','en']:
        run(args.binary, args.output/language, language)


if __name__ == '__main__':
    main()
