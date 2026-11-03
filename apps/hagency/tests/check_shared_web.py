#!/usr/bin/env python3
"""Owner search switch, exact navigation and uncertain-save recovery in the shipped VM."""
import argparse,json,subprocess
from pathlib import Path
from check_full_bundle import fixture,APP
SETUP='''
let H=mod.hagency let A=mod.web_actions
A.redraw=fn(){return nil} H.busy=false H.command=nil H.page="execution_policy" H.policy_mode="detail"
H.entry_role="owner" H.session={account:"@owner:test"} H.policy_resource="resource" H.policy_engagement="agent"
let row={resourceId:"resource" engagementId:"agent" agent:"Yellow" projectId:"" framework:"codex" sharedMode:"yolo" executionPolicy:{yolo:true} webSearch:false appliesTo:"next_dispatch" revision:"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}
H.policy_detail=row
let calls=[] let callbacks=[]
A.submit=fn(service,args){calls.push({service:service args:args})}
A.request=fn(service,args,done){calls.push({service:service args:args}) callbacks.push(done)}
A.save=fn(){return true} let ack=[] A.acknowledge=fn(id){ack.push(id)}
let widgets=[] let visit=fn(node){if !node.is_object(){return} for index key child in node{if key.is_string() || key.is_id(){widgets.push({name:key.to_string() value:child})} visit(child)}}
let find=fn(name){for w in widgets{if w.name==name{return w.value}} return nil}
'''
CASES={
"toggle_preserves_full_trust_and_waits_for_confirmation":'''
A.toggle_policy_web() assert(calls.len()==1 && calls[0].service=="hagency.execution_policy.edit")
let d=calls[0].args assert(d.webSearch && d.executionPolicy.yolo && d.expectedRevision==row.revision)
assert(!row.webSearch)
let result=row.to_json().parse_json() result.webSearch=true
assert(A.valid_policy_result(result,d)) result.webSearch=false assert(!A.valid_policy_result(result,d))
''',
"chat_search_does_not_enable_coding":'''
row.sharedMode="chat" row.executionPolicy.yolo=false A.toggle_policy_web()
assert(calls.len()==1 && calls[0].args.webSearch && !calls[0].args.executionPolicy.yolo)
row.webSearch=true H.policy_mode="result" A.toggle_policy_web() assert(calls.len()==2 && !calls[1].args.webSearch)
''',
"busy_pending_stale_legacy_block":'''
H.busy=true A.toggle_policy_web() H.busy=false H.command={service:"original"} A.toggle_policy_web()
H.command=nil H.policy_engagement="other" A.toggle_policy_web() H.policy_engagement="agent"
row.webSearch=nil A.toggle_policy_web() assert(calls.len()==0)
row.webSearch="yes" assert(!A.policy_valid(row,"resource","agent"))
''',
"lost_response_and_mismatched_response_retain_original":'''
let cmd={service:"hagency.execution_policy.edit" args:{commandId:"original" resourceId:"resource" engagementId:"agent" expectedRevision:row.revision executionPolicy:{yolo:true} webSearch:true}}
H.command=cmd A.policy_reply(cmd,{is_ok:false error:"management_outcome_unknown"}) assert(H.command.args.commandId=="original" && ack.len()==0)
A.policy_reply(cmd,{is_ok:true data:row}) assert(H.command.args.commandId=="original" && ack.len()==0)
row.webSearch=true A.policy_reply(cmd,{is_ok:true data:row}) assert(H.command==nil && H.policy_detail.webSearch && ack[0]=="original")
''',
"permission_save_and_confirmation_preserve_scroll_identity":'''
let route=A.scroll_route()
H.busy=true assert(A.scroll_route()==route) H.busy=false
for mode in ["edit","review","result","detail"] {H.policy_mode=mode assert(A.scroll_route()==route)}
H.policy_engagement="another-agent" assert(A.scroll_route()!=route)
H.policy_engagement="agent" H.policy_resource="another-pool" assert(A.scroll_route()!=route)
H.page="wizard" H.wizard.step=1 let wizard=A.scroll_route() H.wizard.step=2 assert(A.scroll_route()!=wizard)
''',
"full_trust_can_be_changed_after_search_save":'''
H.policy_mode="result" A.choose_policy(false)
assert(H.policy_mode=="edit" && !H.policy_draft.executionPolicy.yolo)
assert(H.policy_draft.expectedRevision==row.revision && row.webSearch==false)
A.review_policy() assert(H.policy_mode=="review") A.submit_policy()
assert(calls.len()==1 && calls[0].args.executionPolicy.yolo==false)
assert(A.recipient_optional(calls[0].args,"webSearch")==nil)
''',

"owner_shortcut_resolves_exact_agent":'''
let r={id:"req" agentId:"agent" poolId:"pool" resourceOwnerMxid:"@owner:test" assignmentId:"assigned"}
H.page="pool_agent" H.pool_detail=r H.pool_meta={conns:[{id:"pool" resourceId:"resource"}]}
A.pool_agent_permissions(r,"") assert(calls.len()==1 && calls[0].service=="hagency.execution_policy.get" && calls[0].args.engagementId=="agent" && calls[0].args.resourceId=="resource")
''',
"manager_cannot_use_owner_shortcut":'''
H.entry_role="manager" A.pool_agent_permissions({resourceOwnerMxid:"@owner:test" assignmentId:"assigned"},"") assert(calls.len()==0)
''',
"cold_shortcut_uses_absent_cursor_then_exact_pool_and_device":'''
let r={id:"req" agentId:"agent" poolId:"pool" resourceOwnerMxid:"@owner:test" assignmentId:"assigned"}
H.page="pool_agent" H.pool_detail=r H.pool_meta=nil H.pool_info=nil H.session.device="device"
A.pool_agent_permissions(r,"") assert(calls.len()==1 && calls[0].args.after==nil)
callbacks[0]({items:[{id:"pool" resourceId:"wrong-device" ownerMxid:"@owner:test" executorDeviceId:"other"}] nextAfter:"next"})
assert(calls.len()==2 && calls[1].args.after=="next")
callbacks[1]({items:[{id:"pool" resourceId:"resource" ownerMxid:"@owner:test" executorDeviceId:"device"}] nextAfter:nil})
assert(calls.len()==3 && calls[2].service=="hagency.execution_policy.get" && calls[2].args.resourceId=="resource" && calls[2].args.engagementId=="agent")
''',
"navigation_ignores_late_response":'''
let r={id:"req" agentId:"agent" poolId:"pool" resourceOwnerMxid:"@owner:test" assignmentId:"assigned"}
H.page="pool_agent" H.pool_detail=r H.pool_meta=nil H.pool_info=nil
A.pool_agent_permissions(r,"") assert(calls.len()==1 && calls[0].service=="hagency.connections.list")
H.page="resources" callbacks[0]({items:[] nextAfter:nil}) assert(calls.len()==1 && H.page=="resources")
''',
}
def main():
 p=argparse.ArgumentParser();p.add_argument('vm',type=Path);p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
 source=(APP/'bundle/main.splash').read_text()
 section=source[source.index('        if H.page=="execution_policy" {'):source.index('        if H.page=="tool_grants" || H.page=="tool_grant_review" {')]
 CASES['actual_switch_renders_and_saves']='''let tree=View{'''+section+'''} visit(tree)
let toggle=find("policy_web_search") assert(toggle!=nil && toggle.enabled && toggle.draw_bg.selected==0)
toggle.on_click() assert(calls.len()==1 && calls[0].args.webSearch)
'''
 CASES['save_feedback_does_not_insert_new_rows']='''let draw=fn(){widgets=[] let tree=View{'''+section+'''} visit(tree)}
draw() let initial=find("policy_save_status") assert(initial!=nil && initial.height==24)
H.busy=true draw() assert(find("policy_save_status").height==initial.height && !find("policy_web_search").enabled)
H.busy=false H.policy_mode="result" draw()
assert(find("policy_save_status").height==initial.height && find("policy_web_search").enabled)
assert(find("policy_result_done")==nil && find("policy_standard").enabled)
'''
 resource=source[source.index('        if H.page=="default_policy" {'):source.index('        if H.page=="',source.index('        if H.page=="default_policy" {')+20)]
 CASES['resource_default_web_switch_renders_and_preserves_permissions']='''
H.page="default_policy" H.dpol.resource="resource" H.dpol.detail={resource:{id:"resource" framework:"codex"}}
H.dpol.mode="catalog" H.dpol.state="ready" H.dpol.yolo=true H.dpol.web=false H.dpol.catalog={resourceId:"resource" revision:"before" agents:[]}
A.web_can_edit=fn(){return true}
host.request=fn(service,args,done){calls.push({service:service args:args}) callbacks.push(done)}
let draw=fn(){widgets=[] let tree=View{'''+resource+'''} visit(tree)}
draw() let toggle=find("default_policy_web_search")
assert(toggle!=nil && toggle.enabled && toggle.draw_bg.selected==0)
toggle.on_click()
assert(calls.len()==1 && calls[0].service=="hagency.web_catalog.prepare")
assert(calls[0].args.change.yolo && calls[0].args.change.webSearch)
draw() assert(!find("default_policy_web_search").enabled && find("default_policy_web_search").draw_bg.selected==0)
'''
 pending=source[source.index('        if H.command != nil && !H.busy && A.sub_page()'):source.index('        if false && H.page=="overview"')]
 CASES['pending_save_keeps_layout_and_uncertain_result_keeps_retry']='''let pending_view=fn(){return View{width:Fill height:Fit
'''+pending+'''}}
H.command={service:"hagency.execution_policy.edit" args:{commandId:"original"}}
for page in ["execution_policy","personal_agent"] {
 H.page=page H.busy=true widgets=[] visit(pending_view()) assert(find("retry_saved")==nil)
 H.busy=false widgets=[] visit(pending_view()) assert(find("retry_saved")!=nil && find("retry_saved").enabled)
}
'''
 results=[]
 for lang in ['en','zh-CN']:
  for name,test in CASES.items():
   path=a.output/f'{lang}-{name}.splash';path.write_text(fixture(source+'\nmod.web_actions=A\n',lang)+SETUP+test+'\ntrue\n')
   r=subprocess.run([str(a.vm.resolve()),'eval',str(path)],stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True);path.with_suffix('.log').write_text(r.stdout);results.append({'language':lang,'case':name,'passed':r.returncode==0})
 (a.output/'report.json').write_text(json.dumps({'checks':results},indent=2)+'\n');print(json.dumps({'passed':sum(r['passed'] for r in results),'total':len(results),'failed':[r['case'] for r in results if not r['passed']]}));raise SystemExit(not all(r['passed'] for r in results))
if __name__=='__main__':main()
