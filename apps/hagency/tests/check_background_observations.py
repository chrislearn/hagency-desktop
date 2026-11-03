#!/usr/bin/env python3
"""Exercise delayed observations against foreground dialogs and durable requests."""
import argparse
import json
import subprocess
from pathlib import Path

from check_full_bundle import APP, fixture

SETUP = r'''
A.redraw=fn(){H.draws=H.draws+1}
H.draws=0 H.calls=[] H.callbacks=[] H.busy=false H.reading=false H.command=nil
H.ready=true H.needs_signin=false H.page="action" H.confirm=""
H.session={account:"@owner:test"} H.entry_role="owner"
H.read_epoch=10 H.webRefresh.visible=true H.webRefresh.inFlight=false
H.selected={contribution:{id:"pool" revision:1}}
A.observe_verification=fn(row,at){}
A.load_connection_capacity=fn(row){}
host.request=fn(service,args,done){H.calls.push({service:service args:args}) H.callbacks.push(done)}
let reply={is_ok:true data:{admission:{contribution:{id:"pool" revision:2}} observedAtMs:100}}
'''

CASES = {
    "confirmation_opened_during_refresh_remains_operable": r'''
A.web_refresh_now()
assert(H.calls.len()==1 && H.webRefresh.inFlight)
assert(!H.busy && H.command==nil)
H.confirm="revoke"
assert(A.alert_kind()=="server_revoke")
H.callbacks[0](reply)
assert(!H.busy && !H.webRefresh.inFlight && H.confirm=="revoke")
assert(H.selected.contribution.revision==1)
''',
    "late_observation_cannot_release_foreground_command_lock": r'''
A.web_refresh_now()
H.read_epoch=H.read_epoch+1 H.busy=true
H.command={service:"hagency.connections.revoke" args:{commandId:"original"}}
H.notice="Submitting original request"
H.callbacks[0](reply)
assert(H.busy && H.command.args.commandId=="original")
assert(H.notice=="Submitting original request" && H.selected.contribution.revision==1)
''',
    "completed_foreground_read_fences_older_observation": r'''
A.web_refresh_now()
H.read_epoch=H.read_epoch+1 H.selected={contribution:{id:"pool" revision:3}}
H.callbacks[0](reply)
assert(!H.busy && H.selected.contribution.revision==3 && H.draws==0)
''',
    "foreground_read_lock_survives_stale_failure": r'''
A.web_refresh_now()
H.read_epoch=H.read_epoch+1 H.busy=true H.reading=true H.notice="Reading current details"
H.callbacks[0]({is_ok:false error:"management_unavailable"})
assert(H.busy && H.reading && H.notice=="Reading current details")
''',
    "hidden_or_replaced_page_drops_observation": r'''
A.web_refresh_now() H.webRefresh.visible=false H.webRefresh.epoch=H.webRefresh.epoch+1
H.callbacks[0](reply)
assert(!H.webRefresh.inFlight && H.selected.contribution.revision==1)
H.webRefresh.visible=true A.web_refresh_now() H.page="overview"
H.callbacks[1](reply)
assert(H.selected.contribution.revision==1 && !H.busy)
''',
    "current_observation_updates_without_duplicate_poll": r'''
A.web_refresh_now() A.web_refresh_now()
assert(H.calls.len()==1 && !H.busy)
H.callbacks[0](reply)
assert(H.selected.contribution.revision==2 && H.draws==1 && !H.webRefresh.inFlight)
''',
    "saved_or_active_mutation_prevents_new_observation": r'''
H.command={service:"original"} A.web_refresh_now()
H.command=nil H.busy=true A.web_refresh_now()
assert(H.calls.len()==0 && !H.webRefresh.inFlight)
''',
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("vm", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    source = (APP / "bundle/main.splash").read_text()
    results = []
    for language in ("en", "zh-CN"):
        for name, case in CASES.items():
            path = args.output / (language + "-" + name + ".splash")
            path.write_text(fixture(source + SETUP + case, language))
            run = subprocess.run([str(args.vm.resolve()), "root", str(path)],
                                 stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
            path.with_suffix(".log").write_text(run.stdout)
            results.append({"case": name, "language": language, "passed": run.returncode == 0})
    (args.output / "report.json").write_text(json.dumps(results, indent=2) + "\n")
    print(json.dumps({"passed": sum(r["passed"] for r in results), "total": len(results)}))
    raise SystemExit(not all(r["passed"] for r in results))


if __name__ == "__main__":
    main()
