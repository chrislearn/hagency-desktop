#!/usr/bin/env python3
"""Native pool deletion eligibility and review; fictional state, no live deletion."""
import argparse
import hashlib
import json
from pathlib import Path
from native_widget_families import CASES, ROOT
from native_owner_tasks import Owner, DETAIL, RESOURCE


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--binary', type=Path, default=ROOT/'target/release/examples/hagency_visual')
    args = parser.parse_args()
    results = []
    for language, narrow in [('en', True), ('zh-CN', True), ('en', False), ('zh-CN', False)]:
        for reason in [None, 'active_grant', 'runtime_not_stopped']:
            name = reason or 'deletable'
            out = args.output.resolve() / (language + ('-narrow-' if narrow else '-wide-') + name)
            catalog = dict(resourceId=RESOURCE['id'], resourceName='Research', revision='revision',
                           rateCapPerDay=None, rateEnforced=False, defaultYolo=False,
                           canRetire=reason is None, retireBlockedReason=reason, agents=[], nextAfter=None)
            web = dict(kind='workforce', after=None, data=None, epoch=0, status='', severity='',
                       agent='', terminalId='', resourceId=RESOURCE['id'], section='settings',
                       catalog=catalog, draft=None, pending=None, phase='', commandId='', rate='',
                       trustRoom='', trustName='', publicationReview=False, readQuery=None, readKind='')
            CASES['retirement'] = dict(page='pool_advanced', entry_role='owner', resource_detail=DETAIL,
                                      resources=[RESOURCE], pool_sub_rid=RESOURCE['id'], pool_adv_state='ready', web=web)
            app = Owner(out, args.binary.resolve(), language, narrow, 'retirement')
            try:
                app.visible('pool_advanced_delete')
                assert app.row('pool_advanced_delete')['enabled'] == (reason is None)
                note = app.row('pool_advanced_delete_note')
                expected = {
                    None: ('unresolved usage', '未结清'),
                    'active_grant': ('authorization', '授权'),
                    'runtime_not_stopped': ('process', '进程'),
                }[reason][language != 'en']
                assert expected in note['t'], note
                width = app.req('/s')['w'][0]['sz'][0]
                assert note['r'][0] >= 0 and note['r'][0] + note['r'][2] <= width
                agents = [r for r in app.snap() if r.get('i') == 'pool_advanced_agents' and r['r'][2] > 0 and r['r'][3] > 0]
                assert bool(agents) == (reason == 'runtime_not_stopped')
                app.shot('eligibility')
                if reason is None:
                    prepared = dict(commandId='offline_retire', resourceId=RESOURCE['id'], expectedRevision='revision',
                                    change=dict(operation='retire'), result=None, error=None)
                    app.set_fixtures({'hagency.web_catalog.prepare': prepared})
                    app.click('pool_advanced_delete')
                    app.visible('web_command_apply')
                    assert app.row('web_command_apply')['enabled']
                    assert any(('未结清' if language != 'en' else 'unresolved usage') in r.get('t', '') for r in app.snap())
                    app.shot('confirmation')
                    # Stop before confirming: this harness checks presentation, not mutation.
                results.append(dict(language=language, narrow=narrow, reason=reason, passed=True))
            except Exception:
                app.shot('failure')
                raise
            finally:
                app.stop()
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output/'report.json').write_text(json.dumps(dict(cases=results, sourceSha256=hashlib.sha256((ROOT/'apps/hagency/bundle/main.splash').read_bytes()).hexdigest()), indent=2))
    print(json.dumps(results))

if __name__ == '__main__':
    main()
