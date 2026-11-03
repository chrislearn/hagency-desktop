#!/usr/bin/env python3
"""Run the shipped pool form and callbacks in the pinned VM; no live writes."""
import argparse
import json
from pathlib import Path
import subprocess
from check_full_bundle import APP, fixture

SETUP = '''
A.redraw=fn(){}
A.pool_sync_amount=fn(){}
A.save=fn(){H.persisted=H.pool_draft.to_json() return true}
H.ready=true H.busy=false H.command=nil H.saved_commands=[] H.pool_checked=true
H.session={account:"@applicant:test" features:["resource-pool-agents-v1"]}
H.page="pool_apply" H.calls=[] H.persisted="" H.ack=""
let pool={id:"pool" label:"Codex" ownerMxid:"@owner:test" canRequest:true blockingReason:nil capabilities:["coding"] revision:7 grantRevision:1 publishedTokens:1000000 nativeAvailableTokens:nil}
H.pool_draft={pool:pool name:"小蓝" description:"Assistant" capability:"coding" tokens:"8000000" roomCommand:"" roomId:"" review:false}
A.request=fn(service,args,done){H.calls.push({service:service args:args}) H.callback=done}
A.submit=fn(service,args){H.calls.push({service:service args:args})}
A.acknowledge=fn(id){H.ack=id}
host.request=fn(service,args,done){H.calls.push({service:service args:args}) H.host_callback=done}
let buttons={}
let button=fn(id){for key value in buttons {if key==id{return value}} return nil}
let collect=fn(value){if !value.is_object(){return} for i key child in value {buttons[key.to_string()]=child collect(child)}}
'''

CASES = {
    "recovery_explains_rooms_that_need_a_fresh_invitation": '''
assert(A.pool_reinvite_notice({})=="")
assert(A.pool_reinvite_notice({roomsRequiringReinvite:0})=="")
let note=A.pool_reinvite_notice({roomsRequiringReinvite:2})
assert(note.search("2")>=0 && note.search(A.t("fresh invitation","重新邀请"))>=0)
''',
    "legacy_publication_renewal_remains_separate_from_agent_renewal": '''
H.resource_detail={id:"legacy-pool"} H.pool_detail=nil
A.pool_view=fn(detail){return {renewOpen:true publishedTokens:1000}}
A.pool_succeed=fn(kind,tokens,people){H.calls.push({kind:kind tokens:tokens})}
A.pool_renew()
assert(H.calls.len()==1 && H.calls[0].kind=="renewal" && H.calls[0].tokens==1000)
''',
    "reauthorization_requires_owner_and_explicit_new_terms": '''
H.pool_detail={id:"old-request" revision:3 resourceOwnerMxid:"@owner:test" canReallocate:true currentGrantRevision:8}
H.pool_confirm="reallocate" H.entry_role="owner" H.session.account="@owner:test"
H.pool_reallocate_mode="" A.pool_reallocate() assert(H.calls.len()==0)
H.pool_reallocate_mode="automatic" A.pool_reallocate()
assert(H.calls.len()==1 && H.calls[0].service=="hagency.pools.agents.reallocate")
assert(H.calls[0].args.requestId=="old-request" && H.calls[0].args.expectedRevision==3 && H.calls[0].args.expectedGrantRevision==8 && H.calls[0].args.renewalMode=="automatic")
H.entry_role="manager" A.pool_reallocate() assert(H.calls.len()==1)
assert(A.pool_renewal({})==A.t("Renewal not selected","续期方式尚未选择"))
''',

    "recurring_request_requires_explicit_mode_and_keeps_owner_calendar": '''
pool.calendar={cadence:"weekly" timeZone:"America/Los_Angeles"}
pool.periodResetLabel="2026-11-09 00:00 PST · America/Los_Angeles"
H.pool_draft.tokens="1000" render()
assert(!button("pool_review").enabled && button("pool_renew_automatic")!=nil && button("pool_renew_owner")!=nil)
button("pool_renew_automatic").on_click() render()
assert(H.pool_draft.renewalMode=="automatic" && button("pool_review").enabled)
H.pool_draft.roomCommand="existing" H.pool_draft.roomId="!existing:test"
A.pool_review() A.pool_send_verified()
assert(H.calls[0].service=="hagency.pools.agents.request" && H.calls[0].args.request.renewalMode=="automatic")
assert(A.recipient_optional(H.calls[0].args.request,"calendar")==nil)
''',
    "recurring_pause_and_owner_renew_are_period_bound": '''
let r={id:"req" revision:9 state:"approved_waiting_executor" resourceOwnerMxid:"@owner:test" reservationState:"reserved" runtimeState:"qualified" blockingReason:nil currentPeriod:{key:"2026-11-02" state:"exhausted"} canRenew:false}
assert(A.local_pool_state(r)==A.t("Paused · allowance used up","已暂停 · 本期额度已用完"))
assert(A.pool_tone(r)=="waiting")
H.pool_detail=r H.pool_confirm="renew" H.entry_role="owner" H.session.account="@owner:test"
A.pool_agent_renew() assert(H.calls.len()==0)
r.canRenew=true r.currentPeriod.state="awaiting_owner"
A.pool_agent_renew() assert(H.calls.len()==1 && H.calls[0].service=="hagency.pools.agents.renew")
assert(H.calls[0].args.expectedRevision==9 && H.calls[0].args.expectedPeriodKey=="2026-11-02")
H.entry_role="manager" A.pool_agent_renew() assert(H.calls.len()==1)
''',
    "recurring_wait_reasons_and_definite_refill_rejection_are_clear": '''
for state in ["awaiting_owner","awaiting_executor","awaiting_capacity","exhausted","revoked"] {
    let label=A.pool_period_state({currentPeriod:{state:state}})
    assert(label!="")
}
assert(A.topup_not_applied("hagency.agents.topups.request","agent_period_allowance_fixed"))
assert(A.pool_reason("agent_period_allowance_fixed").search(A.t("next period","下一周期"))>=0)
assert(A.pool_reason("pool_resource_owner_required").search(A.t("resource owner","owner"))>=0)
''',

    "period_pause_wins_over_stale_runtime_readiness": '''
let r={state:"approved_waiting_executor" reservationState:"reserved" runtimeState:"qualified" membershipState:"matrix_verified" assignmentId:"same-agent" blockingReason:"agent_period_ended"}
assert(A.local_pool_state(r)==A.t("Paused · authorization ended","已暂停 · 本期授权结束"))
assert(A.pool_state(r)==A.t("Paused · waiting for renewed authorization","已暂停 · 等待续期授权"))
assert(A.pool_tone(r)=="waiting")
assert(A.pool_reason(r.blockingReason).search(A.t("rooms and history","房间和历史记录"))>=0)
r.runtimeState="stopped"
assert(A.local_pool_state(r)==A.t("Paused · authorization ended","已暂停 · 本期授权结束"))
r.state="revoked" r.blockingReason=nil
assert(A.local_pool_state(r)==A.t("Stopped","已停止"))
''',
    "selected_pool_replaces_old_unsubmitted_pool": '''
let terra=pool.to_json().parse_json()
terra.id="terra" terra.label="Terra" terra.publishedTokens=5000000 terra.capabilities=["review"]
H.pool_draft.tokens="4000000" H.pool_draft.review=true
A.pool_begin(terra) render()
assert(H.pool_draft.pool.id=="terra" && A.pool_limit(H.pool_draft.pool)==5000000)
assert(H.pool_draft.name=="小蓝" && H.pool_draft.description=="Assistant" && H.pool_draft.tokens=="4000000")
assert(H.pool_draft.capability=="review" && !H.pool_draft.review && H.calls.len()==0)
assert(button("pool_limit").text.search("5 M")>=0 && button("pool_review").enabled)
assert(H.persisted.parse_json().pool.id=="terra")
// Switching back keeps the typed amount and explains the smaller limit.
A.pool_begin(pool) render()
assert(H.pool_draft.pool.id=="pool" && H.pool_draft.tokens=="4000000")
assert(!button("pool_review").enabled && button("pool_validation").text!="")
''',
    "old_lookup_cannot_overwrite_new_selection": '''
A.pool_refresh() let old_reply=H.callback
let terra=pool.to_json().parse_json() terra.id="terra" terra.publishedTokens=5000000
A.pool_begin(terra)
old_reply({items:[pool] nextAfter:nil})
assert(H.pool_draft.pool.id=="terra" && A.pool_limit(H.pool_draft.pool)==5000000 && H.pool_checked)
''',
    "existing_room_and_unresolved_request_cannot_be_retargeted": '''
A.navigate=fn(page){H.page=page}
let terra=pool.to_json().parse_json() terra.id="terra" terra.publishedTokens=5000000
H.pool_draft.roomCommand="original-room" H.pool_draft.roomId="!original:test"
let original=H.pool_draft.to_json()
A.pool_begin(terra)
assert(H.pool_draft.to_json()==original && H.notice.search("Codex")>=0 && H.calls.len()==0)
H.pool_draft.roomCommand="" H.pool_draft.roomId=""
H.saved_commands=[{operation:"pool_agent_request" reply:nil commandId:"saved"}]
original=H.pool_draft.to_json() A.pool_begin(terra)
assert(H.pool_draft.to_json()==original && H.calls.len()==0)
''',
    "missing_name_and_capability_have_visible_reasons": '''
H.pool_draft.tokens="1000" H.pool_draft.name="  " render()
assert(!button("pool_review").enabled && button("pool_validation").visible)
assert(button("pool_validation").text==A.t("Enter an Agent name to continue.","请填写 Agent 名称后继续。"))
H.pool_draft.name="Terra helper" H.pool_draft.capability="invalid" render()
assert(!button("pool_review").enabled && button("pool_validation").text.search(A.t("capability","能力"))>=0)
H.pool_draft.capability="coding" render()
assert(button("pool_review").enabled && !button("pool_validation").visible)
''',
    "over_limit_blocks_before_room": '''
render()
assert(!button("pool_review").enabled && button("pool_amount_error").text!="")
A.pool_review() A.pool_send()
assert(H.calls.len()==0 && !H.pool_draft.review && H.pool_draft.roomCommand=="")
assert(H.pool_draft.tokens=="8000000")
''',
    "slider_presets_and_exact_input": '''
A.pool_ratio(0.25) assert(H.pool_draft.tokens=="250000")
A.pool_ratio(2) assert(H.pool_draft.tokens=="1000000")
A.pool_ratio(-1) assert(H.pool_draft.tokens=="1")
A.pool_tokens("123456") render() assert(button("pool_tokens").text=="123456")
assert(abs(button("pool_slider").value-0.123456)<0.000001 && button("pool_review").enabled)
A.pool_review() assert(H.pool_draft.review && H.calls.len()==0)
''',
    "invalid_and_unknown_limits": '''
for value in ["0","-1","abc","1.5","9007199254740992",""] {
 A.pool_tokens(value) assert(!A.pool_valid(H.pool_draft))
 A.pool_review() A.pool_send() assert(H.calls.len()==0)
}
A.pool_tokens("100") pool.publishedTokens=nil render()
assert(!button("pool_slider").enabled && !button("pool_review").enabled)
pool.publishedTokens=1000000 pool.nativeAvailableTokens=0 render()
assert(!button("pool_slider").enabled && !button("pool_review").enabled)
pool.nativeAvailableTokens=50 assert(A.pool_limit(pool)==50)
assert(!A.pool_valid(H.pool_draft))
''',
    "fresh_limit_before_any_effect": '''
A.pool_tokens("1000000") A.pool_review() A.pool_send()
assert(H.calls.len()==1 && H.calls[0].service=="hagency.pools.list")
let changed=pool.to_json().parse_json() changed.publishedTokens=500000 changed.grantRevision=2
H.callback({items:[changed] nextAfter:nil})
assert(H.calls.len()==1 && !H.pool_draft.review && H.pool_draft.roomCommand=="")
assert(H.pool_draft.tokens=="1000000" && A.pool_limit(H.pool_draft.pool)==500000)
render() assert(!button("pool_review").enabled)
''',
    "paginated_refresh_keeps_draft": '''
H.pool_draft.roomCommand="room-original" H.pool_draft.roomId="!room:test"
A.pool_refresh() H.callback({items:[] nextAfter:"page2"})
assert(H.calls.len()==2 && H.calls[1].args.after=="page2")
H.callback({items:[pool] nextAfter:nil})
assert(H.pool_checked && H.pool_draft.tokens=="8000000" && H.pool_draft.roomId=="!room:test")
''',
    "definitive_rejection_unlocks_existing_room": '''
H.pool_draft.roomCommand="room-original" H.pool_draft.roomId="!room:test" H.pool_draft.review=true
H.command={service:"hagency.pools.agents.request" args:{commandId:"rejected-original"}}
A.retry_command() H.host_callback({is_ok:false error:"pool_agent_resource_mismatch"})
assert(H.command==nil && H.ack=="rejected-original" && !H.pool_draft.review)
assert(H.pool_draft.failure=="pool_agent_resource_mismatch" && H.persisted!="")
assert(A.pool_editable() && H.pool_draft.roomCommand=="room-original" && H.pool_draft.roomId=="!room:test")
A.pool_refresh() H.callback({items:[pool] nextAfter:nil})
A.pool_tokens("1000") A.pool_review() render()
assert(button("pool_edit").enabled && button("pool_submit").enabled)
A.pool_send() H.callback({items:[pool] nextAfter:nil})
let call=H.calls[H.calls.len()-1]
assert(call.service=="hagency.pools.agents.request" && call.args.request.roomId=="!room:test")
for c in H.calls {assert(c.service!="hagency.pools.agents.room")}
''',
    "unknown_result_retains_original_and_locks_edit": '''
H.pool_draft.tokens="1000" H.pool_draft.review=true H.pool_draft.roomCommand="room-original"
H.command={service:"hagency.pools.agents.request" args:{commandId:"unknown-original" requestedTokens:1000}}
let original=H.command.to_json()
A.retry_command() H.host_callback({is_ok:false error:"management_outcome_unknown"})
assert(H.command.to_json()==original && H.ack=="" && !A.pool_editable())
A.pool_tokens("500") A.pool_ratio(0.5) A.pool_edit() A.pool_send()
assert(H.pool_draft.tokens=="1000" && H.pool_draft.review && H.calls.len()==1)
A.retry_command() assert(H.calls[1].args.commandId=="unknown-original")
''',
    "saved_unresolved_request_blocks_new_request": '''
H.saved_commands=[{operation:"pool_agent_request" reply:nil commandId:"saved"}]
A.pool_tokens("1000") A.pool_review() A.pool_send()
assert(!A.pool_editable() && H.calls.len()==0 && H.pool_draft.tokens=="8000000")
''',
}


def main():
    p = argparse.ArgumentParser()
    p.add_argument("vm", type=Path)
    p.add_argument("--output", type=Path, required=True)
    args = p.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    source = (APP / "bundle/main.splash").read_text()
    start = source.index('    if H.page=="pool_apply" && H.pool_draft!=nil{')
    end = source.index('    if H.page=="pool_agents"{', start)
    setup = SETUP
    page = source[start:end]
    results = []
    for language in ("en", "zh-CN"):
        for name, case in CASES.items():
            path = args.output / f"{language}-{name}.splash"
            parts = case.split("render()")
            rendered = parts[0]
            for i, part in enumerate(parts[1:]):
                rendered += f"\nbuttons={{}}\nlet rendered_{i}=View{{\n{page}\n}}\ncollect(rendered_{i})\n" + part
            path.write_text(fixture(source + setup + rendered, language))
            result = subprocess.run([str(args.vm.resolve()), "root", str(path)],
                                    text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            path.with_suffix(".log").write_text(result.stdout)
            results.append({"language": language, "case": name, "passed": result.returncode == 0})
    (args.output / "report.json").write_text(json.dumps(results, indent=2) + "\n")
    print(json.dumps({"passed": sum(r["passed"] for r in results), "total": len(results)}))
    raise SystemExit(not all(r["passed"] for r in results))


if __name__ == "__main__":
    main()
