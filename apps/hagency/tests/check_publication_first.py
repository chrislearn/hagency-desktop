#!/usr/bin/env python3
"""One native publication confirmation, before the administrator's decision."""
import argparse
import json
import subprocess
from pathlib import Path
from check_full_bundle import APP, fixture

SETUP = r'''
A.redraw=fn(){} A.save=fn(){return true}
H.ready=true H.busy=false H.command=nil H.needs_signin=false
H.session={account:"@owner:test" device:"device" features:["pool-publication-first-v1"]}
H.wizard={label:"My pool" resource:{resourceId:"resource" revision:"revision"}}
let c={id:"pool" resourceId:"resource" ownerMxid:"@owner:test" executorDeviceId:"device" state:"draft" publicationFirst:true}
'''
CASES = {
    "initial_submit_requests_private_draft_with_existing_identity": r'''
A.submit=fn(service,args){H.called={service:service args:args}}
A.contribute()
assert(H.called.service=="hagency.connections.create")
assert(H.called.args.publicationFirst && H.called.args.resourceId=="resource")
assert(H.called.args.coordinatorMxid=="@owner:test")
''',
    "one_native_confirmation_then_read_current_admission": r'''
H.calls=[]
A.pool_keep_summary=fn(id,v){H.kept=id}
A.open_connection=fn(id){H.read=id}
A.request=fn(service,args,done){H.calls.push({service:service args:args}) H.done=done}
A.publish_offer(c)
assert(H.calls.len()==1 && H.calls[0].service=="hagency.grants.review")
assert(H.calls[0].args.contributionId=="pool")
assert(H.calls[0].args.applicants[0]=="@owner:test")
H.done({state:"server_observed" serverState:"active"})
assert(H.read=="pool" && H.kept=="pool")
''',
    "cancel_does_not_submit_a_second_admission": r'''
H.calls=[]
A.pool_keep_summary=fn(id,v){} A.open_connection=fn(id){H.read=id}
A.request=fn(service,args,done){H.calls.push(service) H.done=done}
A.publish_offer(c) H.done({state:"review_closed"})
assert(H.calls.len()==1 && H.read=="pool" && c.state=="draft")
''',
    "approved_or_submitted_pool_never_reopens_owner_confirmation": r'''
H.calls=0 A.request=fn(service,args,done){H.calls+=1}
c.state="pending_admission" A.publish_offer(c)
c.state="admitted" A.publish_offer(c)
assert(H.calls==0)
''',
    "draft_and_pending_have_different_next_actions": r'''
let draft=A.pool_banner({connState:"ready" conn:c})
assert(draft.kind=="draft" && draft.step==1)
c.state="pending_admission"
let submitted=A.pool_banner({connState:"ready" conn:c})
assert(submitted.kind=="pending" && submitted.step==2)
assert(A.publish_steps_text(2)==A.t("Step 2 of 2 · Administrator approval","第 2 步，共 2 步 · 管理员批准"))
''',
    "legacy_saved_commands_keep_old_wire_and_no_automatic_funding": r'''
H.session.features=[]
A.submit=fn(service,args){H.args=args}
A.contribute()
assert(A.recipient_optional(H.args,"publicationFirst")==nil)
c.publicationFirst=false
H.calls=0 A.request=fn(service,args,done){H.calls+=1}
A.publish_offer(c)
assert(H.calls==0)
''',
}

def main():
    p=argparse.ArgumentParser();p.add_argument("vm",type=Path);p.add_argument("--output",type=Path,required=True);a=p.parse_args()
    a.output.mkdir(parents=True,exist_ok=True);source=(APP/"bundle/main.splash").read_text();results=[]
    for language in ("en","zh-CN"):
        for name,case in CASES.items():
            path=a.output/(language+"-"+name+".splash");path.write_text(fixture(source+SETUP+case,language))
            r=subprocess.run([str(a.vm.resolve()),"root",str(path)],stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
            path.with_suffix(".log").write_text(r.stdout);results.append({"case":name,"language":language,"passed":r.returncode==0})
    (a.output/"report.json").write_text(json.dumps(results,indent=2)+"\n")
    print(json.dumps({"passed":sum(r["passed"] for r in results),"total":len(results)}))
    raise SystemExit(not all(r["passed"] for r in results))

if __name__=="__main__":main()
