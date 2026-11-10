use super::*;

// ADR-186 on the console routes: the approval chooses its amount (§A), the
// quota pause shows on the reads (§B), and a top-up raises a running
// engagement's allocation and lifts the pause (§C).

fn allocated_column(state: &std::path::Path, id: &str) -> Option<u64> {
    rusqlite::Connection::open(state.join("domain.sqlite3"))
        .unwrap()
        .query_row(
            "SELECT allocated_tokens FROM engagements WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
}

/// §A1: approving with a lower amount reserves exactly that amount.
#[tokio::test]
async fn native_allocation_route_approves_a_lower_amount() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let state = f.root.path().join("state");
    let pending = f.new_engagement_requesting("allocation_lower", 300).await;
    let cookie = lifecycle_session(&service).await;
    // A zero amount and an unknown key are invalid bodies, decided before
    // any store job.
    for body in [
        json!({"commandId": "cmd_lower_zero", "allocatedTokens": 0}),
        json!({"commandId": "cmd_lower_extra", "allocatedTokens": 10, "extra": 1}),
        json!({"commandId": "cmd_lower_negative", "allocatedTokens": -5}),
    ] {
        let refused = post(
            &format!("/console/api/engagements/{pending}/approve"),
            &cookie,
        )
        .json(&body)
        .send(&service)
        .await;
        assert_eq!(refused.status_code, Some(StatusCode::BAD_REQUEST), "{body}");
    }
    let mut response = post(
        &format!("/console/api/engagements/{pending}/approve"),
        &cookie,
    )
    .json(&json!({"commandId": "cmd_lower", "allocatedTokens": 40}))
    .send(&service)
    .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let body = response.take_json::<Value>().await.unwrap();
    assert_eq!(body["state"], "reserved");
    assert_eq!(body.as_object().unwrap().len(), 3, "the bounded receipt");
    assert_eq!(allocated_column(&state, &pending), Some(40));
    f.close().await;
}

/// §A2/§A3: the candidate read's `remainingTokens` is the approval's own
/// headroom: approving exactly that ("All remaining") succeeds, one token
/// more is refused with `over_commit` and the human message.
#[tokio::test]
async fn native_allocation_route_all_remaining_and_named_refusal() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let state = f.root.path().join("state");
    let pending = f.new_engagement_requesting("allocation_all", 100).await;
    let cookie = lifecycle_session(&service).await;
    let mut listed = get(
        &format!("/console/api/engagements/{pending}/candidates"),
        &cookie,
    )
    .send(&service)
    .await;
    assert_eq!(listed.status_code, Some(StatusCode::OK));
    let body = listed.take_json::<Value>().await.unwrap();
    let remaining = body["candidates"][0]["remainingTokens"].as_u64().unwrap();
    // The 1000-token pool holds the seeded UsageWorker's 100.
    assert_eq!(remaining, 900);
    let mut over = post(
        &format!("/console/api/engagements/{pending}/approve"),
        &cookie,
    )
    .json(&json!({"commandId": "cmd_all_over", "allocatedTokens": remaining + 1}))
    .send(&service)
    .await;
    assert_eq!(over.status_code, Some(StatusCode::CONFLICT));
    let refusal = over.take_json::<Value>().await.unwrap();
    assert_eq!(refusal["code"], "over_commit");
    let message = refusal["message"]
        .as_str()
        .expect("the human message travels");
    assert!(message.contains("would exceed"), "{message}");
    assert!(message.contains("NewUsageWorker"), "{message}");
    let ok = post(
        &format!("/console/api/engagements/{pending}/approve"),
        &cookie,
    )
    .json(&json!({"commandId": "cmd_all", "allocatedTokens": remaining}))
    .send(&service)
    .await;
    assert_eq!(ok.status_code, Some(StatusCode::OK));
    assert_eq!(allocated_column(&state, &pending), Some(900));
    f.close().await;
}

/// §B: a quota hold shows on Engagements (`quotaPaused`, with the allocation
/// and the known spend) and on Workforce (`quota_paused`). The hold row is
/// written directly: the store tests own how it opens, this pins the reads.
#[tokio::test]
async fn native_allocation_route_reads_show_the_quota_pause() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let state = f.root.path().join("state");
    let cookie = lifecycle_session(&service).await;
    let paused = |value: &Value| -> Option<bool> {
        value["engagements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == f.engagement.as_str())
            .map(|row| row["quotaPaused"].as_bool().unwrap())
    };
    let mut listed = get("/console/api/engagements", &cookie)
        .send(&service)
        .await;
    let before = listed.take_json::<Value>().await.unwrap();
    assert_eq!(paused(&before), Some(false));
    let row = before["engagements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == f.engagement.as_str())
        .unwrap()
        .clone();
    assert_eq!(row["allocatedTokens"], 100, "the request is the allocation");
    // The seed's period is incomplete (an empty snapshot between two
    // counts) but carries counts: its known lower bound is the spend.
    assert!(
        row["spentTokens"].as_u64().is_some(),
        "the seeded usage is counted"
    );
    rusqlite::Connection::open(state.join("domain.sqlite3"))
        .unwrap()
        .execute(
            "INSERT INTO quota_holds(engagement_id,dispatch_id,spend,allocation,began_at) VALUES(?1,NULL,100,100,1)",
            [&f.engagement],
        )
        .unwrap();
    let mut listed = get("/console/api/engagements", &cookie)
        .send(&service)
        .await;
    assert_eq!(
        paused(&listed.take_json::<Value>().await.unwrap()),
        Some(true)
    );
    let mut roster = get("/console/api/agents", &cookie).send(&service).await;
    assert_eq!(roster.status_code, Some(StatusCode::OK));
    let roster = roster.take_json::<Value>().await.unwrap();
    let agent = roster["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["engagement_id"] == f.engagement.as_str())
        .unwrap()
        .clone();
    assert_eq!(agent["quota_paused"], true);
    f.close().await;
}

/// §C: the top-up route. Checked like an approval, idempotent by command
/// id, and a hold the new allocation clears is lifted.
#[tokio::test]
async fn native_allocation_route_top_up_lifts_the_pause_and_is_idempotent() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let state = f.root.path().join("state");
    let id = f.engagement.clone();
    let path = format!("/console/api/engagements/{id}/allocation");
    let cookie = lifecycle_session(&service).await;
    for body in [
        json!({"commandId": "cmd_top_zero", "addTokens": 0}),
        json!({"commandId": "cmd_top_missing"}),
        json!({"commandId": "cmd_top_extra", "addTokens": 5, "allocatedTokens": 5}),
    ] {
        let refused = post(&path, &cookie).json(&body).send(&service).await;
        assert_eq!(refused.status_code, Some(StatusCode::BAD_REQUEST), "{body}");
    }
    // The seeded engagement is paused (a hold written directly; the store
    // tests own how one opens).
    rusqlite::Connection::open(state.join("domain.sqlite3"))
        .unwrap()
        .execute(
            "INSERT INTO quota_holds(engagement_id,dispatch_id,spend,allocation,began_at) VALUES(?1,NULL,100,100,1)",
            [&id],
        )
        .unwrap();
    // The 1000-token pool holds this engagement's 100: 900 can be added.
    let mut over = post(&path, &cookie)
        .json(&json!({"commandId": "cmd_top_over", "addTokens": 901}))
        .send(&service)
        .await;
    assert_eq!(over.status_code, Some(StatusCode::CONFLICT));
    let refusal = over.take_json::<Value>().await.unwrap();
    assert_eq!(refusal["code"], "over_commit");
    assert!(
        refusal["message"]
            .as_str()
            .unwrap()
            .contains("would exceed")
    );
    assert_eq!(
        allocated_column(&state, &id),
        None,
        "a refusal changes nothing"
    );
    let mut response = post(&path, &cookie)
        .json(&json!({"commandId": "cmd_top", "addTokens": 50}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let body = response.take_json::<Value>().await.unwrap();
    assert_eq!(body["id"], id);
    assert_eq!(body["state"], "active");
    assert_eq!(body.as_object().unwrap().len(), 3, "the bounded receipt");
    assert_eq!(allocated_column(&state, &id), Some(150));
    // The replay answers the same receipt and raises nothing.
    let replay = post(&path, &cookie)
        .json(&json!({"commandId": "cmd_top", "addTokens": 50}))
        .send(&service)
        .await;
    assert_eq!(replay.status_code, Some(StatusCode::OK));
    assert_eq!(allocated_column(&state, &id), Some(150));
    let mut changed = post(&path, &cookie)
        .json(&json!({"commandId": "cmd_top", "addTokens": 60}))
        .send(&service)
        .await;
    assert_eq!(changed.status_code, Some(StatusCode::CONFLICT));
    assert_eq!(
        changed.take_json::<Value>().await.unwrap()["code"],
        "decision_conflict"
    );
    // 150 is above the seeded spend: the hold lifted.
    let mut listed = get("/console/api/engagements", &cookie)
        .send(&service)
        .await;
    let listed = listed.take_json::<Value>().await.unwrap();
    let row = listed["engagements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == id.as_str())
        .unwrap()
        .clone();
    assert_eq!(row["quotaPaused"], false);
    assert_eq!(row["allocatedTokens"], 150);
    assert_eq!(row["requestedTokens"], 100, "the ask is kept");
    // A pending engagement holds no allocation to raise.
    let pending = f.new_engagement_requesting("allocation_pending", 10).await;
    let mut refused = post(
        &format!("/console/api/engagements/{pending}/allocation"),
        &cookie,
    )
    .json(&json!({"commandId": "cmd_top_pending", "addTokens": 5}))
    .send(&service)
    .await;
    assert_eq!(refused.status_code, Some(StatusCode::CONFLICT));
    assert_eq!(
        refused.take_json::<Value>().await.unwrap()["code"],
        "engagement_not_live"
    );
    f.close().await;
}
