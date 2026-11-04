#!/usr/bin/env python3
"""Run/Stop UI semantics from the shipped bundle; no HTTP or provider effects."""
import argparse,json,subprocess
from pathlib import Path
from check_full_bundle import fixture,APP
SETUP='''
let H=mod.hagency let A=mod.control_actions
A.redraw=fn(){return nil} H.busy=false H.ready=true H.notice="" H.command=nil
H.session={account:"@manager:test" features:["shared-run-control-v1"]}
let calls=[] let callbacks=[]
A.request=fn(service,args,done){calls.push({service:service args:args}) callbacks.push(done)}
A.submit=fn(service,args){calls.push({service:service args:args})}
H.control_assignment="mas_original" H.control_agent_name="Original agent"
H.agent_control={readiness:{control:{assignmentId:"mas_original" revision:7 mode:"paused"} canRun:true canStop:true runBlockingReason:nil intakeEnabled:true automaticDeliveryEnabled:true} observedAtUtc:"2026-10-16 12:00:00 UTC"}
let widgets=[] let visit=fn(node){if !node.is_object(){return} for index key child in node{if key.is_string() || key.is_id(){widgets.push({name:key.to_string() value:child})} visit(child)}}
let find=fn(name){for row in widgets{if row.name==name{return row.value}} return nil}
'''
CASES={
'local_pause_pending_keeps_original_request_without_claiming_remote_success': '''
let callbacks=[] host.request=fn(service,args,done){calls.push({service:service args:args}) callbacks.push(done)}
A.save=fn(){return true} let acknowledgements=[] A.acknowledge=fn(id){acknowledgements.push(id)}
H.command={service:"hagency.agents.control.set" args:{commandId:"saved_stop" assignmentId:"mas_original" expectedRevision:7 mode:"paused"}}
A.retry_command() callbacks[0]({is_ok:false error:"management_agent_control_local_stop_pending"})
assert(H.command.args.commandId=="saved_stop" && H.command.args.expectedRevision==7 && H.agent_control==nil)
assert(acknowledgements.len()==0 && H.notice==A.control_reason("management_agent_control_local_stop_pending"))
assert(H.notice!=A.error_text("management_agent_control_local_stop_pending"))
A.retry_command() assert(calls.len()==2 && calls[1].args.commandId=="saved_stop" && calls[1].args.expectedRevision==7)
''',
'lost_reply_retries_original_id_and_then_reads_latest_setting': '''
let callbacks=[] host.request=fn(service,args,done){calls.push({service:service args:args}) callbacks.push(done)}
A.save=fn(){return true} A.acknowledge=fn(id){assert(id=="saved_stop")}
H.command={service:"hagency.agents.control.set" args:{commandId:"saved_stop" assignmentId:"mas_original" expectedRevision:7 mode:"paused"}}
A.retry_command() callbacks[0]({is_ok:false error:"management_outcome_unknown"})
assert(H.command.args.commandId=="saved_stop" && H.command.args.expectedRevision==7 && H.agent_control==nil)
A.retry_command() assert(calls.len()==2 && calls[1].args.commandId=="saved_stop" && calls[1].args.expectedRevision==7)
callbacks[1]({is_ok:true data:{commandId:"saved_stop" appliedRevision:8 control:{assignmentId:"mas_original" revision:9 mode:"running"}}})
assert(H.command==nil && calls.len()==3 && calls[2].service=="hagency.agents.control.readiness")
''',
'recovery_returns_latest_assignment_without_a_new_mutation': '''
A.save=fn(){return true} A.acknowledge=fn(id){assert(id=="saved_stop")}
A.recover_native({commandId:"saved_stop" operation:"agent_run_control"})
assert(calls.len()==1 && calls[0].service=="hagency.intent.retry")
callbacks[0]({operation:"agent_run_control" data:{control:{assignmentId:"mas_original" revision:9 mode:"running"}}})
assert(H.page=="agent_control" && H.command==nil && calls.len()==2 && calls[1].service=="hagency.agents.control.readiness")
''',
'closed_original_assignment_and_revision': '''
A.set_agent_control("running") A.set_agent_control("paused") A.set_agent_control("arbitrary")
assert(calls.len()==2 && calls[0].service=="hagency.agents.control.set" && calls[0].args.assignmentId=="mas_original" && calls[0].args.expectedRevision==7 && calls[0].args.mode=="running" && calls[1].args.mode=="paused")
''',
'owner_stop_only_and_disabled_intake': '''
H.agent_control.readiness.canRun=false H.agent_control.readiness.intakeEnabled=false H.agent_control.readiness.automaticDeliveryEnabled=false
A.set_agent_control("running") A.set_agent_control("paused")
assert(calls.len()==1 && calls[0].args.mode=="paused")
''',
'pending_or_stale_record_cannot_replace_original': '''
H.command={service:"original" args:{commandId:"original-id"}} A.set_agent_control("running") A.set_agent_control("paused")
assert(calls.len()==0 && H.command.args.commandId=="original-id")
H.command=nil H.agent_control.readiness.control.assignmentId="other" A.set_agent_control("running") assert(calls.len()==0)
''',
'wrong_assignment_response_is_not_displayed': '''
let wrong={readiness:{control:{assignmentId:"other"}}}
A.open_agent_control("mas_original","Original agent")
assert(calls.len()==1 && calls[0].args.assignmentId=="mas_original" && H.agent_control==nil)
callbacks[0](wrong) assert(H.agent_control==nil && H.notice!="")
''',
'older_server_does_not_issue_control_requests': '''
H.session.features=[] A.open_agent_control("mas_original","Original agent") A.set_agent_control("running") A.set_agent_control("paused") assert(calls.len()==0)
''',
'control_refresh_reads_setting_and_execution_without_writes': '''
H.page="agent_control" H.pool_detail={id:"par_original" assignmentId:"mas_original" runtimeState:"issued"}
H.control_poll_pool=false
let first=A.web_refresh_plan()
assert(first.service=="hagency.agents.control.readiness" && first.args.assignmentId=="mas_original")
first.done(H.agent_control)
let second=A.web_refresh_plan()
assert(second.service=="hagency.pools.agents.get" && second.args.id=="par_original")
second.done({request:{id:"par_original" assignmentId:"mas_original" runtimeState:"stopped"}})
assert(H.pool_detail.runtimeState=="stopped" && !H.control_poll_pool && calls.len()==0)
H.control_assignment="mas_other" first.done({readiness:{control:{assignmentId:"mas_original"}}})
assert(!H.control_poll_pool && calls.len()==0)
''',
'pool_execution_state_does_not_fall_back_to_approval': '''
let r={state:"approved_waiting_executor" reservationState:"reserved" runtimeState:"stopped" membershipState:"matrix_verified" blockingReason:nil}
assert(A.local_pool_state(r)==A.t("Execution stopped","执行已停止"))
r.runtimeState="issued" assert(A.local_pool_state(r)==A.t("Starting","正在启动"))
r.runtimeState="qualified" assert(A.local_pool_state(r)==A.t("Process started · last reported","进程已启动 · 最近记录"))
r.runtimeState="failed" assert(A.local_pool_state(r)==A.t("Start failed","启动失败") && A.pool_tone(r)=="attention")
r.runtimeState="outcome_unknown" assert(A.local_pool_state(r)==A.t("Execution status unknown","执行状态未知") && A.pool_tone(r)=="attention")
''',
'control_setting_never_claims_process_is_running': '''
assert(A.control_mode_text("running")!="Running" && A.control_mode_text("paused")!="Stopped")
assert(A.control_reason("already_running")=="")
'''
}
def main():
 p=argparse.ArgumentParser();p.add_argument('vm',type=Path);p.add_argument('--bundle',type=Path,default=APP/'bundle/main.splash');p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);source=a.bundle.read_text()
 start=source.index('        if H.page == "agent_control" {mod.widgets.HagencyDetailCard{');end=source.index('\n        if H.page ==',start+10);section=source[start:end]
 CASES['role_controls_render_and_preserve_exact_cas']='''
H.page="agent_control"
let tree=View{'''+section+'''} visit(tree)
assert(find("control_run")!=nil && find("control_pause")!=nil)
find("control_run").on_click() assert(calls.len()==1 && calls[0].args.expectedRevision==7)
widgets=[] H.agent_control.readiness.canRun=false
let owner=View{'''+section+'''} visit(owner)
assert(find("control_run")==nil && find("control_pause")!=nil)
'''
 CASES['pending_result_renders_no_new_controls']='''
H.page="agent_control" H.command={service:"original" args:{commandId:"saved"}}
let tree=View{'''+section+'''} visit(tree)
assert(find("control_run")==nil && find("control_pause")==nil && calls.len()==0)
'''
 CASES['pooled_response_settings_return_to_the_same_agent']='''
H.page="agent_control" H.pool_detail={id:"pool_request_original" assignmentId:"mas_original" state:"approved_waiting_executor" reservationState:"reserved" runtimeState:"qualified" membershipState:"matrix_verified" blockingReason:nil}
A.open_pool_agent=fn(id){calls.push({openedAgent:id})}
let tree=View{'''+section+'''} visit(tree)
assert(find("control_back").text=="Original agent")
find("control_back").on_click()
assert(calls.len()==1 && calls[0].openedAgent=="pool_request_original")
'''
 results=[]
 for lang in ['en','zh-CN']:
  for name,test in CASES.items():
   path=a.output/f'{lang}-{name}.splash';path.write_text(fixture(source+'\nmod.control_actions=A\n',lang)+SETUP+test+'\ntrue\n')
   r=subprocess.run([str(a.vm.resolve()),'eval',str(path)],stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True);path.with_suffix('.log').write_text(r.stdout);results.append({'language':lang,'case':name,'passed':r.returncode==0})
 (a.output/'report.json').write_text(json.dumps({'checks':results},indent=2)+'\n');print(json.dumps({'passed':sum(r['passed'] for r in results),'total':len(results)}));raise SystemExit(not all(r['passed'] for r in results))
if __name__=='__main__':main()
