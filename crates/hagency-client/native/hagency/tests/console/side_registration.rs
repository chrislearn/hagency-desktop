use super::*;

// Task #13: the retained `POST /api/project-sides/:id/registration-file`
// (`backend-v2.js:10031-10168`) on the native console. The assertions are
// the TS-visible outcomes: the response's key set and values (camelCase,
// fingerprints, never tokens), the YAML file's field-for-field shape
// (`renderRegistrationYaml`, `appservice-receiver.js:111-136`), the
// stored token at the exact path the appservice profile reads
// (`bootstrap/config.rs:646`), and TS's staging behaviour on a reissue.

fn yaml_field(yaml: &str, key: &str) -> String {
    // `trim_start()`: namespaced keys (`rooms:`) are indented two spaces.
    yaml.lines()
        .find_map(|l| l.trim_start().strip_prefix(&format!("{key}: ")))
        .expect("key present in the YAML")
        .trim_matches('"')
        .to_owned()
}

#[tokio::test]
async fn native_console_issue_side_registration_matches_ts() {
    let f = Fixture::new("127.0.0.1:13300".parse().unwrap());
    let service = f.service();
    let state = f.root.path().join("state");

    // TS parity (#31): there is no read-only login — one login is the whole
    // console. An anonymous caller is refused before any store read (the
    // console's authenticate hoop rejects it without a cookie); every
    // logged-in session may issue the registration file.
    let anonymous = TestClient::post(format!(
        "{BASE}/console/api/project-sides/example.test/registration-file"
    ))
    .add_header("host", "127.0.0.1:13300", true)
    .add_header("origin", BASE, true)
    .add_header("sec-fetch-site", "same-origin", true)
    .json(&json!({"url": "http://127.0.0.1:13443"}))
    .send(&service)
    .await;
    assert_eq!(anonymous.status_code, Some(StatusCode::UNAUTHORIZED));

    // Ticket issuance is rate-limited to one per second.
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let cookie = lifecycle_session(&service).await;

    // TS refusals: 404 for an unknown side, 400 for a missing url.
    let missing = post(
        "/console/api/project-sides/nowhere.test/registration-file",
        &cookie,
    )
    .json(&json!({"url": "http://127.0.0.1:13443"}))
    .send(&service)
    .await;
    assert_eq!(missing.status_code, Some(StatusCode::NOT_FOUND));
    let no_url = post(
        "/console/api/project-sides/example.test/registration-file",
        &cookie,
    )
    .json(&json!({"url": "   "}))
    .send(&service)
    .await;
    assert_eq!(no_url.status_code, Some(StatusCode::BAD_REQUEST));

    // THE FIRST ISSUE. Defaults only, exactly the TS endpoint's defaults.
    let mut response = post(
        "/console/api/project-sides/example.test/registration-file",
        &cookie,
    )
    .json(&json!({"url": "http://127.0.0.1:13443///"}))
    .send(&service)
    .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let body: Value = response.take_json().await.unwrap();
    // The TS key set — `ok` plus the camelCase body — and nothing else.
    // (serde_json's default map is ordered, so compare sorted.)
    let mut keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(|k| k.as_str())
        .collect();
    keys.sort_unstable();
    let mut expected = [
        "staged",
        "path",
        "mode",
        "registrationId",
        "senderLocalpart",
        "representative",
        "namespace",
        "url",
        "asTokenFingerprint",
        "hsTokenFingerprint",
        "nextSteps",
        "ok",
    ];
    expected.sort_unstable();
    assert_eq!(keys, expected);
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["staged"], json!(false));
    assert_eq!(body["mode"], json!("0600"));
    assert_eq!(body["registrationId"], json!("hagency-example.test"));
    assert_eq!(body["senderLocalpart"], json!("hagency"));
    assert_eq!(body["representative"], json!("@hagency:example.test"));
    assert_eq!(body["namespace"], json!("@ac_.*"));
    // Trailing slashes stripped, as `generateRegistration` does.
    assert_eq!(body["url"], json!("http://127.0.0.1:13443"));
    for key in ["asTokenFingerprint", "hsTokenFingerprint"] {
        let value = body[key].as_str().unwrap();
        assert_eq!(value.len(), 8);
        assert!(value.bytes().all(|b| b.is_ascii_hexdigit()));
    }
    assert_eq!(body["nextSteps"].as_array().unwrap().len(), 5);
    // No staged note on a first issue, exactly as TS renders it.
    assert!(body.get("stagedNote").is_none());

    // The YAML file: at the TS-declared path, field-for-field.
    let yaml_path = state.join("registrations/example.test.yaml");
    let yaml = std::fs::read_to_string(&yaml_path).unwrap();
    assert_eq!(yaml_field(&yaml, "id"), "hagency-example.test");
    assert_eq!(yaml_field(&yaml, "url"), "http://127.0.0.1:13443");
    assert_eq!(yaml_field(&yaml, "sender_localpart"), "hagency");
    assert_eq!(yaml_field(&yaml, "rate_limited"), "false");
    assert_eq!(yaml_field(&yaml, "rooms"), "[]");
    assert!(
        yaml.contains("  users:\n    - exclusive: true\n      regex: \"@ac_.*\"\n  aliases: []\n")
    );
    let as_token = yaml_field(&yaml, "as_token");
    let hs_token = yaml_field(&yaml, "hs_token");
    assert_eq!(as_token.len(), 64);
    assert_eq!(hs_token.len(), 64);
    assert_ne!(as_token, hs_token);

    // The token the SERVICE reads: at the exact path `bootstrap/config.rs`
    // reads for the appservice profile — no hand-placement left between
    // issuing and serving — and equal to the YAML's as_token.
    let stored_token =
        String::from_utf8(std::fs::read(state.join("matrix.appservice_token")).unwrap()).unwrap();
    assert_eq!(stored_token, as_token);
    // The store's own read returns the same live credential.
    let credential = f
        .domain
        .side_credential_for_transport("example.test".into())
        .await
        .unwrap()
        .expect("live credential present");
    assert_eq!(credential.as_token, as_token);
    assert_eq!(credential.hs_token, hs_token);
    assert_eq!(credential.namespace, "@ac_.*");
    assert_eq!(credential.sender_localpart, "hagency");

    // No token bytes anywhere in the HTTP response — the ADR-132 negative.
    let serialized = serde_json::to_string(&body).unwrap();
    assert!(!serialized.contains(&as_token));
    assert!(!serialized.contains(&hs_token));

    // THE REISSUE STAGES: the live credential (and the token the service
    // reads) is untouched until a verify promotes the staged one.
    let mut second = post(
        "/console/api/project-sides/example.test/registration-file",
        &cookie,
    )
    .json(&json!({"url": "http://127.0.0.1:14443"}))
    .send(&service)
    .await;
    assert_eq!(second.status_code, Some(StatusCode::OK));
    let second: Value = second.take_json().await.unwrap();
    assert_eq!(second["staged"], json!(true));
    assert_eq!(
        second["stagedNote"].as_str().unwrap(),
        "The credential this side is USING has not changed. This new one is held until you \
         install it and verification proves the homeserver accepts it, so nothing breaks in the \
         meantime — and if you generated it by mistake, ignore the file and nothing happens."
    );
    // The staged YAML replaced the file; the SERVICE's token did not move.
    let staged_yaml = std::fs::read_to_string(&yaml_path).unwrap();
    let staged_as_token = yaml_field(&staged_yaml, "as_token");
    assert_ne!(staged_as_token, as_token);
    let still_live =
        String::from_utf8(std::fs::read(state.join("matrix.appservice_token")).unwrap()).unwrap();
    assert_eq!(still_live, as_token);
    let credential = f
        .domain
        .side_credential_for_transport("example.test".into())
        .await
        .unwrap()
        .expect("live credential still present");
    assert_eq!(credential.as_token, as_token);

    f.close().await;
}
