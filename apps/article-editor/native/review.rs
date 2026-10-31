//! Offline instrumentation identity; no sync, real credentials or publication.
//! The editor still uses its normal consent, storage and input handlers.
pub fn install_offline_account() -> Result<(), String> {
    let root = crate::app_data_dir();
    if !root.join(".article-review-profile").is_file() {
        return Err("An isolated article review profile is required".into());
    }
    if let Some(client) = crate::sliding_sync::get_client() {
        return if client.user_id().is_some_and(|id| id.as_str() == "@article-review:example.invalid") {
            Ok(()) // Stylesheet reload preserves the same account and consent.
        } else {
            Err("Refusing to replace an existing account".into())
        };
    }
    crate::sliding_sync::backend_runtime().block_on(async {
        let client = matrix_sdk::Client::builder()
            .homeserver_url("http://127.0.0.1:9")
            .server_versions([matrix_sdk::ruma::api::MatrixVersion::V1_0])
            .build().await.map_err(|e| e.to_string())?;
        client.matrix_auth().restore_session(serde_json::from_value(serde_json::json!({
            "user_id":"@article-review:example.invalid", "device_id":"OFFLINE_ARTICLE_REVIEW",
            "access_token":"offline-fixture-not-a-credential"
        })).map_err(|e| e.to_string())?, Default::default()).await.map_err(|e| e.to_string())?;
        crate::sliding_sync::replace_client(Some(client));
        Ok(())
    })
}
