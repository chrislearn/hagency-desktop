//! Exercise the real SDK device store and signature verification over HTTP.
use super::*;
use matrix_sdk::{
    SessionMeta, SessionTokens,
    authentication::matrix::MatrixSession,
    encryption::LocalTrust,
    ruma::{api::MatrixVersion, device_id, user_id},
};
use matrix_sdk_crypto::{OlmMachine, types::requests::AnyOutgoingRequest};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

struct KeysServer {
    origin: String,
    queries: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl KeysServer {
    fn new(keys: Value, initially_empty: bool) -> Self {
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", socket.local_addr().unwrap());
        socket.set_nonblocking(true).unwrap();
        let queries = Arc::new(AtomicUsize::new(0));
        let seen = queries.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let thread = std::thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                let Ok((mut stream, _)) = socket.accept() else {
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut block = [0; 4096];
                let (head, body) = loop {
                    let n = stream.read(&mut block).unwrap_or(0);
                    if n == 0 || bytes.len() > 65_536 {
                        break (String::new(), Vec::new());
                    }
                    bytes.extend_from_slice(&block[..n]);
                    if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8(bytes[..i].to_vec()).unwrap();
                        let length = head
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|s| s.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= i + 4 + length {
                            break (head, bytes[i + 4..i + 4 + length].to_vec());
                        }
                    }
                };
                if head.is_empty() {
                    continue;
                }
                let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let querying_agent = head.lines().next().unwrap().contains("/keys/query ")
                    && body["device_keys"].get("@_hagency_agt_one:test").is_some();
                let response = if querying_agent {
                    let n = seen.fetch_add(1, Ordering::AcqRel);
                    if initially_empty && n == 0 {
                        json!({"device_keys":{}, "failures":{}})
                    } else {
                        json!({"device_keys":{"@_hagency_agt_one:test":{"AGENT":keys}},"failures":{}})
                    }
                } else if head.contains("/keys/query ") {
                    json!({"device_keys":{}, "failures":{}})
                } else if head.contains("/keys/upload ") {
                    json!({"one_time_key_counts":{"signed_curve25519":50}})
                } else {
                    json!({})
                };
                let body = response.to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
            }
        });
        Self {
            origin,
            queries,
            stop,
            thread: Some(thread),
        }
    }

    async fn client(&self) -> Client {
        let client = Client::builder()
            .homeserver_url(&self.origin)
            .server_versions([MatrixVersion::V1_11])
            .build()
            .await
            .unwrap();
        client
            .restore_session(MatrixSession {
                meta: SessionMeta {
                    user_id: user_id!("@owner:test").to_owned(),
                    device_id: device_id!("OWNER").to_owned(),
                },
                tokens: SessionTokens {
                    access_token: "local-fixture".into(),
                    refresh_token: None,
                },
            })
            .await
            .unwrap();
        client
    }
}

impl Drop for KeysServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
    }
}

async fn signed_device_keys() -> Value {
    let machine = OlmMachine::new(user_id!("@_hagency_agt_one:test"), device_id!("AGENT")).await;
    machine
        .outgoing_requests()
        .await
        .unwrap()
        .iter()
        .find_map(|request| {
            if let AnyOutgoingRequest::KeysUpload(upload) = request.request() {
                upload
                    .device_keys
                    .as_ref()
                    .map(|keys| serde_json::to_value(keys).unwrap())
            } else {
                None
            }
        })
        .unwrap()
}

#[tokio::test]
async fn encrypted_agent_first_send_queries_again_after_empty_device_list() {
    let server = KeysServer::new(signed_device_keys().await, true);
    let client = server.client().await;
    tokio::time::timeout(
        Duration::from_secs(5),
        wait_for_device(&client, user_id!("@_hagency_agt_one:test")),
    )
    .await
    .expect("must discover newly published device")
    .unwrap();
    assert!(server.queries.load(Ordering::Acquire) >= 2);
    let devices = client
        .encryption()
        .get_user_devices(user_id!("@_hagency_agt_one:test"))
        .await
        .unwrap();
    let device = devices.devices().next().unwrap();
    assert_eq!(device.device_id(), device_id!("AGENT"));
    // Do not change the user's device policy just to make a send succeed.
    device
        .set_local_trust(LocalTrust::BlackListed)
        .await
        .unwrap();
    let result = tokio::time::timeout(
        Duration::from_millis(700),
        wait_for_device(&client, user_id!("@_hagency_agt_one:test")),
    )
    .await;
    assert!(
        !matches!(result, Ok(Ok(()))),
        "blacklisted keys must not authorize an encrypted send"
    );
}

#[tokio::test]
async fn encrypted_agent_first_send_rejects_invalid_device_signature() {
    let mut keys = signed_device_keys().await;
    keys["keys"]["curve25519:AGENT"] = json!("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
    let server = KeysServer::new(keys, false);
    let client = server.client().await;
    let result = tokio::time::timeout(
        Duration::from_millis(700),
        wait_for_device(&client, user_id!("@_hagency_agt_one:test")),
    )
    .await;
    assert!(
        !matches!(result, Ok(Ok(()))),
        "invalid keys must not authorize an encrypted send"
    );
    assert!(server.queries.load(Ordering::Acquire) > 0);
    assert_eq!(
        client
            .encryption()
            .get_user_devices(user_id!("@_hagency_agt_one:test"))
            .await
            .unwrap()
            .devices()
            .count(),
        0
    );
}
