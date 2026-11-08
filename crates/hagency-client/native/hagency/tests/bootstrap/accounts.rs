use super::*;
#[cfg(unix)]
#[tokio::test]
async fn native_bootstrap_local_codex() {
    use std::os::unix::fs::PermissionsExt;
    for kind in [
        "valid", "profile", "extra", "managed", "factory", "missing", "writable",
    ] {
        let f = Fixture::new(false).await;
        let root = f.root.path().canonicalize().unwrap();
        let home = root.join("provider-home");
        let codex = root.join("provider-codex");
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir(&codex).unwrap();
        let path = f.state_dir.join("development-driver.json");
        let mut config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        config["local_codex"] = json!({"profile":"provider_owned_codex_v1","preset":"pool","seat":"seat","home":home,"codex_home":codex});
        match kind {
            "profile" => config["local_codex"]["profile"] = json!("implicit"),
            "extra" => config["local_codex"]["ready"] = json!(true),
            "managed" => config["managed_account"] = json!("account_cannot_fallback"),
            "factory" => {
                config["factory_service"] =
                    json!({"profile":"inline_factory_service_checkpoint_v1","idle_ms":1000})
            }
            "missing" => config["local_codex"]["codex_home"] = json!(root.join("missing")),
            "writable" => {
                std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o777)).unwrap()
            }
            _ => {}
        }
        std::fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
        let result = hagency::bootstrap::Bootstrap::open(&f.state_dir, f.address, 16, true);
        if kind == "valid" {
            result.unwrap().close().await.unwrap();
        } else {
            assert!(
                matches!(result, Err(hagency::bootstrap::Failure::Config { .. })),
                "{kind}"
            );
        }
        assert!(!f.state_dir.join("runtime-home").exists());
        assert_eq!(f.attempts(), 0);
        assert_eq!(f.fake.requests(), 0);
        assert_eq!(std::fs::read_dir(&home).unwrap().count(), 0);
        assert_eq!(std::fs::read_dir(&codex).unwrap().count(), 0);
        f.fake.close().await;
    }
}
