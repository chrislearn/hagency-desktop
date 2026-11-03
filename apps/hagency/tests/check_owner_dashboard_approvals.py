#!/usr/bin/env python3
"""Exercise real dashboard navigation callbacks without approving any request."""
import argparse
import json
import subprocess
from pathlib import Path
from check_full_bundle import APP, fixture

CHECKS = r'''
let calls=[]
host.request=fn(service,args,done){calls.push({service:service args:args done:done})}
A.redraw=fn(){}
H.ready=true H.restored=true H.session={account:"@owner:test" canAdmitResources:false}
H.busy=false H.command=nil H.page="overview" H.entry_role="manager"
on_owner_approvals_requested("@other:test","wrong-owner")
assert(H.page=="overview" && H.owner_review_pending==nil)
assert(calls.len()==1 && calls[0].args.destination=="owner_approvals_rejected")
assert(calls[0].args.requestId=="wrong-owner")
calls.clear()
H.command={id:"existing-command"}
on_owner_approvals_requested("@owner:test","busy-command")
assert(H.page=="overview" && H.command.id=="existing-command")
assert(calls.len()==1 && calls[0].args.destination=="owner_approvals_rejected")
calls.clear() H.command=nil
H.busy=true H.reading=false
on_owner_approvals_requested("@owner:test","busy-write")
assert(H.page=="overview" && calls[0].args.destination=="owner_approvals_rejected")
calls.clear() H.busy=false
H.session=nil
on_owner_approvals_requested("@owner:test","startup")
assert(calls.len()==0 && H.owner_review_pending.nonce=="startup")
H.session={account:"@owner:test" canAdmitResources:false}
A.owner_review_open()
assert(H.page=="owner_approvals" && H.entry_role=="owner" && !H.entry_choosing)
assert(H.owner_review_pending==nil && calls.len()==1)
assert(calls[0].service=="hagency.navigation.open" && calls[0].args.destination=="owner_approvals_ready")
assert(calls[0].args.requestId=="startup")
let landing=View{A.render_page()}
// A second ready callback cannot replay the handover.
A.owner_review_open()
assert(calls.len()==1)
calls.clear()
// Stale resource filters must not hide approvals in another resource pool.
H.tools_resource="stale" H.policy_resource="stale-policy" H.tool_resource="stale-tool" H.tool_engagement="stale-agent" H.tool_after="old-cursor"
A.owner_review_tools()
assert(H.page=="tool_approvals" && H.tools_resource=="" && H.policy_resource=="" && H.tool_resource=="" && H.tool_engagement=="" && H.tool_after=="")
assert(calls.len()==1 && calls[0].service=="hagency.shared_approvals.list")
calls[0].done({is_ok:true data:{approvals:[{canDecide:true decision:nil},{canDecide:false decision:nil},{canDecide:true decision:"approve_once"}]}})
assert(A.shared_rows(true).len()==1 && A.shared_rows(false).len()==1)
// Global permissions have no per-resource tool page. They must still render
// loading/empty/error states without dereferencing a missing resource result.
H.shared_approvals=[]
for state in ["loading","ready","error"] {
    H.shared_state=state
    let permissions=View{A.render_page()}
}
calls.clear()
// Agent requests use the server's needsMyAction query across ALL owner pools.
H.owner.poolId="old-pool" H.owner.poolLabel="Old pool" H.reqs.filter="all"
A.owner_review_agents()
assert(H.page=="inbox" && H.owner.poolId=="" && H.reqs.filter=="pending")
assert(calls.len()==1 && calls[0].service=="hagency.pools.agents.list")
assert(calls[0].args.needsMyAction==true && calls[0].args.after==nil)
let {poolId=nil}=calls[0].args
assert(poolId==nil)
// Every request above is read-only or navigation; no decision is submitted.
assert(H.command==nil)
'''

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("vm", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    source = (APP / "bundle/main.splash").read_text()
    results = []
    for language in ["en", "zh-CN"]:
        path = args.output / (language + ".splash")
        path.write_text(fixture(source + CHECKS, language))
        result = subprocess.run([str(args.vm.resolve()), "root", str(path)], text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        path.with_suffix(".log").write_text(result.stdout)
        results.append({"language": language, "passed": result.returncode == 0})
    (args.output / "report.json").write_text(json.dumps(results, indent=2) + "\n")
    print(json.dumps(results))
    raise SystemExit(not all(result["passed"] for result in results))

if __name__ == "__main__":
    main()
