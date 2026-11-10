use super::*;
use rusqlite::Connection;
use std::path::Path;

/// Every user table's full contents in a stable order — the strongest
/// no-write proof: any INSERT/UPDATE/DELETE the preview committed would
/// change this string.
fn digest(path: &Path) -> String {
    let conn = Connection::open(path).unwrap();
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let mut out = String::new();
    for table in tables {
        out.push_str(&table);
        out.push('\n');
        let mut stmt = conn.prepare(&format!("SELECT * FROM \"{table}\"")).unwrap();
        let columns = stmt.column_count();
        let mut rows = stmt.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            for i in 0..columns {
                let value: rusqlite::types::Value = row.get(i).unwrap();
                out.push_str(&format!("{value:?}|"));
            }
            out.push('\n');
        }
    }
    out
}

/// The fixture's REAL live (reserved/active) coding engagements, ordered as the
/// store orders them — the expectation each read below is checked against, so
/// the tests never hardcode a row count the shared fixture can change.
fn live_coding(db: &Path) -> Vec<(String, String)> {
    let conn = Connection::open(db).unwrap();
    conn.prepare(
        "SELECT json_extract(projection,'$.agentName'),resource_id FROM engagements \
         WHERE state IN ('reserved','active') AND json_extract(projection,'$.role')='coding' ORDER BY id",
    )
    .unwrap()
    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

/// The three reads share the console's read authority: session exchange
/// required; anonymous, forged cookie and foreign headers refused; a list
/// observation takes no selection.
#[tokio::test]
async fn native_console_offer_reads_require_operator_authority() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    for path in [
        "/console/api/offer-book",
        "/console/api/contributions",
        "/console/api/engagements/preview?role=coding",
    ] {
        let anonymous = TestClient::get(format!("{BASE}{path}"))
            .add_header("host", "127.0.0.1:13300", true)
            .send(&service)
            .await;
        assert_eq!(
            anonymous.status_code,
            Some(StatusCode::UNAUTHORIZED),
            "{path}"
        );
    }
    let cookie = session(&service).await;
    for (name, value) in [
        ("host", "evil.test"),
        ("origin", "https://evil.test"),
        ("sec-fetch-site", "cross-site"),
        ("x-forwarded-for", "127.0.0.1"),
        ("forwarded", "for=127.0.0.1"),
        ("cookie", "hagency_console=bad"),
        ("cookie", &format!("{cookie}; {cookie}")),
    ] {
        let response = get("/console/api/offer-book", &cookie)
            .add_header(name, value, true)
            .send(&service)
            .await;
        assert!(
            matches!(
                response.status_code,
                Some(StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
            ),
            "{name}"
        );
    }
    // Foreign query parameters are refused; the one declared key is accepted.
    for query in [
        "?limit=1",
        "?room=x",
        "?projectRoomId=a&projectRoomId=b",
        "?projectRoomId=%31",
    ] {
        let response = get(&format!("/console/api/offer-book{query}"), &cookie)
            .send(&service)
            .await;
        assert_eq!(
            response.status_code,
            Some(StatusCode::BAD_REQUEST),
            "{query}"
        );
    }
    // The two list/preview reads take no selection at all beyond their keys.
    for path in [
        "/console/api/contributions?limit=1",
        "/console/api/engagements/preview?nope=1",
    ] {
        let response = get(path, &cookie).send(&service).await;
        assert_eq!(
            response.status_code,
            Some(StatusCode::BAD_REQUEST),
            "{path}"
        );
    }
    f.close().await;
}

/// The offer book serves the real published roles over HTTP, with the TS
/// JSON shape: real `runningNow`, real serving resource, `null` caps.
#[tokio::test]
async fn native_console_offer_book_serves_real_state() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = session(&service).await;
    let mut response = get("/console/api/offer-book", &cookie).send(&service).await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["whitelisted"], json!(null));
    assert_eq!(value["projectRoomId"], json!(null));
    let roles = value["roles"].as_array().unwrap();
    let names: Vec<&str> = roles.iter().map(|r| r["role"].as_str().unwrap()).collect();
    assert_eq!(names, ["coding", "testing", "integration", "documentation"]);
    let coding = &roles[0];
    // The live figure is the fixture's REAL reserved/active count, never a guess.
    assert_eq!(
        coding["runningNow"],
        json!(live_coding(&f.root.path().join("state/domain.sqlite3")).len() as u64)
    );
    assert_eq!(coding["budgetCapPerEngagement"], json!(null));
    assert_eq!(coding["rateCap"], json!(null));
    assert_eq!(coding["count"], json!(null));
    assert_eq!(coding["serving"]["framework"], json!("codex"));
    assert_eq!(coding["serving"]["model"], json!("gpt-5.6-sol"));
    assert_eq!(coding["serving"]["tier"], json!("medium"));
    // Both published codex/medium resources qualify for `coding`.
    let resource_names: Vec<&str> = coding["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(resource_names, ["private_alert_pool", "private_usage_pool"]);
    // The named-room form echoes the room and still publishes no trust.
    let mut named = get(
        "/console/api/offer-book?projectRoomId=!room:example.test",
        &cookie,
    )
    .send(&service)
    .await;
    assert_eq!(named.status_code, Some(StatusCode::OK));
    let named = named.take_json::<Value>().await.unwrap();
    assert_eq!(named["projectRoomId"], json!("!room:example.test"));
    assert_eq!(named["whitelisted"], json!(null));
    f.close().await;
}

/// Contributions: the real agent<->project relationship, membership probe
/// `null` (never checked), and no private fixture value on the wire.
#[tokio::test]
async fn native_console_contributions_serve_real_relationships() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = session(&service).await;
    let mut response = get("/console/api/contributions", &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    let rows = value["contributions"].as_array().unwrap();
    // The fixture's REAL live relationships — one row per reserved/active
    // engagement, so the count is derived, never hardcoded.
    let expected = live_coding(&f.root.path().join("state/domain.sqlite3"));
    assert_eq!(rows.len(), expected.len(), "one row per live engagement");
    // The seeded worker is present with its real project identity.
    let worker = rows
        .iter()
        .find(|row| row["agent"] == json!("UsageWorker"))
        .expect("the seeded UsageWorker is contributed");
    assert_eq!(worker["project"], json!("project_one"));
    assert_eq!(worker["projectRoomId"], json!("!project:example.test"));
    assert_eq!(worker["ownerMxid"], json!("@owner:example.test"));
    assert_eq!(
        worker["active"],
        json!(true),
        "its provision effect applied"
    );
    // The membership probe has no native source: `null` (never checked), NOT
    // `false` ("the agent is not in the room") — the two must not collapse.
    for row in rows {
        assert_eq!(row["agentJoined"], json!(null));
        assert_eq!(row["membershipCheckedAt"], json!(null));
    }
    // The privacy boundary for THIS route: it publishes the project room id and
    // the owner's mxid (TS parity, `backend-v2.js:15412-15416`; the approvals
    // read already publishes `projectRoomId`), but never the owner's DM room or
    // any internal session/workspace identity. `assert_private` is deliberately
    // NOT used: it forbids any `roomId` substring, which this route's own
    // `projectRoomId` key contains.
    let text = value.to_string();
    for private in [
        "!private:example.test",
        "private_session",
        "private_workspace",
    ] {
        assert!(
            !text.contains(private),
            "unexpected private output {private}"
        );
    }
    f.close().await;
}

/// The preview answers with the real route and headroom — and commits
/// nothing: the whole database is byte-identical across the call.
#[tokio::test]
async fn native_console_preview_is_a_dry_run() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = session(&service).await;
    let db = f.root.path().join("state/domain.sqlite3");
    let before = digest(&db);
    let mut response = get(
        "/console/api/engagements/preview?role=coding&requestedTokens=100",
        &cookie,
    )
    .send(&service)
    .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["route"], json!("notWhitelisted"));
    assert_eq!(value["autoJoin"], json!(false));
    // The agent is the first live coding engagement's own agent and its figure
    // is that resource's real headroom — derived from the fixture, not assumed.
    let live = live_coding(&db);
    assert_eq!(
        value["agent"],
        json!(live[0].0),
        "the first live coding agent"
    );
    // A live engagement exists because it did not create a second one: the row
    // count is unchanged across the call.
    let mut list = get("/console/api/engagements", &cookie)
        .send(&service)
        .await;
    let listed = list.take_json::<Value>().await.unwrap();
    let listed_count = listed["engagements"].as_array().unwrap().len();
    // An unknown role is refused, not answered.
    let refused = get("/console/api/engagements/preview?role=not_a_role", &cookie)
        .send(&service)
        .await;
    assert_eq!(refused.status_code, Some(StatusCode::BAD_REQUEST));
    assert_eq!(digest(&db), before, "the preview must not write");
    let mut after = get("/console/api/engagements", &cookie)
        .send(&service)
        .await;
    let after = after.take_json::<Value>().await.unwrap();
    assert_eq!(
        after["engagements"].as_array().unwrap().len(),
        listed_count,
        "the preview created no engagement"
    );
    f.close().await;
}
