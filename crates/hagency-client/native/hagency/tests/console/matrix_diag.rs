use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

/// A raw HTTP response with a correct Content-Length, so reqwest reads the
/// body and closes instead of waiting for a chunk terminator.
fn http_response(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// A mock homeserver that answers one connection per entry with the given
/// response. Bound to an ephemeral 127.0.0.1 port (the sandbox permits
/// localhost binds per RULES).
fn mock_homeserver(responses: Vec<String>) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for response in responses {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            stream.set_read_timeout(Some(Duration::from_secs(3))).ok();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    addr
}

/// A port with no listener (connect refusal), for the connect-failure arm.
fn closed_port() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    addr
}

/// TS `probeHomeserver` (lib/matrix-candidates.js:115): a 200 with a
/// `versions` array is the ONLY thing counted reachable, and the answer
/// carries the last three versions.
#[tokio::test]
async fn native_matrix_probe_success() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = session(&service).await;
    let addr = mock_homeserver(vec![http_response(
        "200 OK",
        "{\"versions\":[\"v1.1\",\"v1.2\",\"v1.3\",\"v1.4\"]}",
    )]);
    let mut response = post("/console/api/matrix/probe", &cookie)
        .json(&json!({ "url": format!("http://{addr}") }))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["origin"], json!(format!("http://{addr}")));
    assert_eq!(value["via"], json!("you gave the address explicitly"));
    assert_eq!(value["probe"]["reachable"], json!(true));
    assert_eq!(value["probe"]["status"], json!(200));
    assert_eq!(value["probe"]["versions"], json!(["v1.2", "v1.3", "v1.4"]));
    f.close().await;
}

/// TS probe: a 401 (or any non-200) is reported as `reachable:false` with
/// the status and the verbatim reason — never misread as reachable.
#[tokio::test]
async fn native_matrix_probe_401() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = session(&service).await;
    let addr = mock_homeserver(vec![http_response(
        "401 Unauthorized",
        "{\"errcode\":\"M_UNKNOWN_TOKEN\"}",
    )]);
    let mut response = post("/console/api/matrix/probe", &cookie)
        .json(&json!({ "url": format!("http://{addr}") }))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["probe"]["reachable"], json!(false));
    assert_eq!(value["probe"]["status"], json!(401));
    assert_eq!(
        value["probe"]["reason"],
        json!("homeserver answered HTTP 401")
    );
    f.close().await;
}

/// TS probe: a connection failure is a reachable:false with the connect
/// reason (the route still answers 200 — the probe verdict is the payload).
#[tokio::test]
async fn native_matrix_probe_connect_failure() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = session(&service).await;
    let addr = closed_port();
    let mut response = post("/console/api/matrix/probe", &cookie)
        .json(&json!({ "url": format!("http://{addr}") }))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["probe"]["reachable"], json!(false));
    assert_eq!(
        value["probe"]["reason"],
        json!("could not connect to the homeserver")
    );
    f.close().await;
}

/// TS probe 400 arm: neither a name nor a url is a `server_name or url is
/// required`.
#[tokio::test]
async fn native_matrix_probe_requires_input() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = session(&service).await;
    let mut response = post("/console/api/matrix/probe", &cookie)
        .json(&json!({}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::BAD_REQUEST));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["error"], json!("server_name or url is required"));
    f.close().await;
}

/// TS `reach` (backend-v2.js:9698): the fixture's one registered side appears
/// as a homeserver candidate. Native's ProjectSide carries no apiBaseUrl, so
/// the address is null and the probe is TS's "no probeable URL" arm — honest,
/// never an outbound DNS guess.
#[tokio::test]
async fn native_matrix_reach_reports_sides() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = session(&service).await;
    let mut response = get("/console/api/matrix/reach", &cookie)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    let servers = value["homeservers"].as_array().unwrap();
    assert_eq!(servers.len(), 1, "the fixture registers one fleet side");
    let side = &servers[0];
    assert_eq!(side["serverName"], json!("example.test"));
    assert!(side["url"].is_null());
    assert_eq!(side["source"], json!("already recorded as a project side"));
    assert_eq!(side["alreadyASide"], json!(true));
    assert_eq!(side["hasCredential"], json!(false));
    assert_eq!(side["probe"]["reachable"], json!(false));
    assert_eq!(
        side["probe"]["reason"],
        json!("no probeable URL: a bare server name is not an address")
    );
    // Native configures no appservice port / edge / sync intake: the honest
    // no-config arm, not a guessed inbound path.
    assert_eq!(value["appservice"]["listening"], json!(false));
    assert_eq!(value["appservice"]["port"], Value::Null);
    assert_eq!(value["appservice"]["inboundVia"], Value::Null);
    assert!(
        value["appservice"]["reason"]
            .as_str()
            .unwrap()
            .starts_with("HAGENCY_APPSERVICE_PORT is not set")
    );
    f.close().await;
}

/// TS `callback-check` (backend-v2.js:9643): native configures no appservice
/// port / edge, so `verifyCallbackFromHomeserver`'s "nothing to reach" arm is
/// the answer — `applicable:false` with its verbatim reason.
#[tokio::test]
async fn native_matrix_callback_check_no_port() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let cookie = session(&service).await;
    let mut response = post("/console/api/matrix/callback-check", &cookie)
        .json(&json!({ "homeserver_url": "https://example.test" }))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(value["applicable"], json!(false));
    assert_eq!(
        value["reason"],
        json!(
            "neither an appservice port nor a co-located edge is configured, so there is nothing for your homeserver to reach"
        )
    );
    f.close().await;
}
