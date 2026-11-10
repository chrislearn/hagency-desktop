use super::*;

/// Task #12's console surface: `GET /console/api/matrix/pending-invites`
/// and `POST …/decide` — the retained `backend-v2.js:10819-10855` shapes.
/// Asserts the TS-visible outcomes: the list's exact key set (the
/// `pending-invite-store.js` eight camelCase keys), the scope gate, the
/// 400/404/409 refusals, the `queued:true` response, and that an accept
/// places the join on the poller's worklist.
const ROOM: &str = "!dm:example.test";
const AGENT: &str = "Worker";

#[tokio::test]
async fn native_console_pending_invites_list_and_decide() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let read_only = session(&service).await;
    let mut response = get("/console/api/matrix/pending-invites", &read_only)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let body: Value = response.take_json().await.unwrap();
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["pending"], 0);
    assert_eq!(body["invites"].as_array().unwrap().len(), 0);

    // Seed one pending invitation through the store the poller owns.
    f.domain
        .remember_pending_invite(
            ROOM.into(),
            AGENT.into(),
            Some("@stranger:example.test".into()),
            "direct".into(),
            42,
        )
        .await
        .unwrap();
    let mut response = get("/console/api/matrix/pending-invites", &read_only)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let body: Value = response.take_json().await.unwrap();
    assert_eq!(body["pending"], 1);
    let row = &body["invites"][0];
    // The TS backend's exact eight keys, camelCase, and their values.
    let keys: Vec<&str> = row
        .as_object()
        .unwrap()
        .keys()
        .map(|k| k.as_str())
        .collect();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    let mut expected = [
        "projectRoomId",
        "agent",
        "inviter",
        "projectServer",
        "state",
        "seenAt",
        "decidedAt",
        "decidedBy",
    ];
    expected.sort_unstable();
    assert_eq!(sorted, expected);
    assert_eq!(row["projectRoomId"], json!(ROOM));
    assert_eq!(row["agent"], json!(AGENT));
    assert_eq!(row["inviter"], json!("@stranger:example.test"));
    assert_eq!(row["projectServer"], json!("example.test"));
    assert_eq!(row["state"], json!("pending"));
    assert_eq!(row["decidedAt"], Value::Null);
    assert_eq!(row["decidedBy"], Value::Null);

    // One login carries the decide permission (the operator's one-login
    // decision); anonymous callers are refused by the shared hoop.
    let cookie = lifecycle_session(&service).await;

    // TS 400s: room and agent are required.
    for body in [
        json!({"agent": AGENT}),
        json!({"projectRoomId": ROOM}),
        json!({"projectRoomId": "", "agent": AGENT}),
    ] {
        let bad = post("/console/api/matrix/pending-invites/decide", &cookie)
            .json(&body)
            .send(&service)
            .await;
        assert_eq!(bad.status_code, Some(StatusCode::BAD_REQUEST));
    }
    // TS 404s: an unknown invitation.
    let missing = post("/console/api/matrix/pending-invites/decide", &cookie)
        .json(&json!({"projectRoomId": "!none:example.test", "agent": AGENT, "accept": true}))
        .send(&service)
        .await;
    assert_eq!(missing.status_code, Some(StatusCode::NOT_FOUND));

    // The accept: `queued:true` — the decision recorded, the join owed to
    // the poller, never claimed done here.
    let mut accepted = post("/console/api/matrix/pending-invites/decide", &cookie)
        .json(&json!({"projectRoomId": ROOM, "agent": AGENT, "accept": true}))
        .send(&service)
        .await;
    assert_eq!(accepted.status_code, Some(StatusCode::OK));
    let body: Value = accepted.take_json().await.unwrap();
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["queued"], json!(true));
    assert_eq!(body["invite"]["state"], json!("accepted"));
    assert_eq!(body["invite"]["decidedBy"], json!("operator"));
    // The worklist carries the join for the next poll round.
    assert_eq!(
        f.domain
            .join_pending_invites(AGENT.into())
            .await
            .unwrap()
            .len(),
        1
    );
    // The list no longer shows it: pending-only.
    let mut response = get("/console/api/matrix/pending-invites", &read_only)
        .send(&service)
        .await;
    let body: Value = response.take_json().await.unwrap();
    assert_eq!(body["pending"], 0);

    // TS 409s: `already_{state}` for a decided invitation.
    let mut conflict = post("/console/api/matrix/pending-invites/decide", &cookie)
        .json(&json!({"projectRoomId": ROOM, "agent": AGENT, "accept": false}))
        .send(&service)
        .await;
    assert_eq!(conflict.status_code, Some(StatusCode::CONFLICT));
    let text = conflict.take_string().await.unwrap();
    assert!(text.contains("already_accepted"), "{text}");

    f.close().await;
}
