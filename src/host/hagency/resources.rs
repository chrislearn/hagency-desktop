//! Closed resource detail read; configuration authority is intentionally absent.
use super::{parse, Value};
use ::hagency::native_owner::ResourceDetailQuery;

pub(super) const RENAME: &str = "hagency.resources.rename";

pub(super) fn parse_rename(
    service: &str,
    args: Value,
) -> Result<Option<::hagency::native_owner::RenameResource>, String> {
    if service != RENAME {
        return Ok(None);
    }
    let input: ::hagency::native_owner::RenameResource = parse(args)?;
    input
        .validate()
        .map_err(|_| "invalid_management_arguments".to_owned())?;
    Ok(Some(input))
}

pub(super) const GET: &str = "hagency.resources.get";

pub(super) fn parse_get(service: &str, args: Value) -> Result<Option<ResourceDetailQuery>, String> {
    if service != GET {
        return Ok(None);
    }
    let query: ResourceDetailQuery = parse(args)?;
    query
        .validate()
        .map_err(|_| "invalid_management_arguments".to_owned())?;
    Ok(Some(query))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::hagency::{permissions, ManagementPermission};
    use crate::miniapps::InstanceId;
    use serde_json::json;
    use std::time::{Duration, Instant};

    #[test]
    fn resource_name_host_input_and_permission_are_separate_from_configuration() {
        let valid = json!({"commandId":"original", "resourceId":"resource_1", "expectedNameRevision":3, "name":"写作"});
        assert_eq!(
            parse_rename(RENAME, valid.clone())
                .unwrap()
                .unwrap()
                .expected_name_revision,
            3
        );
        for field in [
            "owner",
            "profile",
            "device",
            "provider",
            "path",
            "url",
            "monthlyTokens",
            "capabilities",
            "model",
            "expectedRevision",
        ] {
            let mut input = valid.clone();
            input[field] = json!("forged");
            assert!(parse_rename(RENAME, input).is_err());
        }
        for name in ["x".repeat(81), " leading".into(), "two\nlines".into()] {
            let mut input = valid.clone();
            input["name"] = json!(name);
            assert!(parse_rename(RENAME, input).is_err());
        }
        let mut clear = valid;
        clear["name"] = json!("");
        assert!(parse_rename(RENAME, clear).is_ok());
        let authority = rinx_miniapp_core::SessionAuthority::default();
        let lease = authority.issue(
            InstanceId {
                app: octosense_app_contract::hagency::APP_ID.into(),
                account: "@owner:test".into(),
                room: None,
                generation: 1,
            },
            [RENAME.to_owned()].into_iter().collect(),
            Default::default(),
            Instant::now() + Duration::from_secs(60),
        );
        let granted = permissions(&lease);
        assert!(granted.contains(&ManagementPermission::ReadResources));
        assert!(granted.contains(&ManagementPermission::RenameResources));
        assert_eq!(granted.len(), 2);
        assert!(octosense_app_contract::hagency::SERVICES.contains(&RENAME));
        assert!(octosense_app_contract::KNOWN_CAPABILITIES.contains(&RENAME));
        assert!(octosense_app_contract::hagency::words(RENAME).is_some());
    }

    #[test]
    fn resource_detail_host_input_is_closed_before_manager_initialization() {
        let valid = json!({"id":"resource_1"});
        assert_eq!(
            parse_get(GET, valid.clone()).unwrap().unwrap().id,
            "resource_1"
        );
        for field in [
            "owner",
            "device",
            "provider",
            "path",
            "url",
            "canEdit",
            "create",
            "commandId",
            "expectedRevision",
        ] {
            let mut input = valid.clone();
            input[field] = json!("forged");
            assert!(parse_get(GET, input).is_err());
        }
        for id in ["".into(), "x".repeat(129), "../resource".into()] {
            assert!(parse_get(GET, json!({"id":id})).is_err());
        }
        assert!(parse_get("unknown", json!({})).unwrap().is_none());
    }

    #[test]
    fn resource_detail_screen_requests_only_the_read_grant() {
        let authority = rinx_miniapp_core::SessionAuthority::default();
        let identity = InstanceId {
            app: octosense_app_contract::hagency::APP_ID.into(),
            account: "@owner:test".into(),
            room: None,
            generation: 1,
        };
        let lease = authority.issue(
            identity,
            [GET.to_owned()].into_iter().collect(),
            Default::default(),
            Instant::now() + Duration::from_secs(60),
        );
        assert_eq!(
            permissions(&lease),
            vec![ManagementPermission::ReadResources]
        );
        assert!(octosense_app_contract::hagency::SERVICES.contains(&GET));
        assert!(octosense_app_contract::KNOWN_CAPABILITIES.contains(&GET));
        assert!(octosense_app_contract::hagency::words(GET).is_some());
        // Reading a detail grants no mutations. The complete owner app also
        // declares publication separately for its explicit publish flow.
        let manifest: Value =
            serde_json::from_str(include_str!("../../../apps/hagency/bundle/manifest.json"))
                .unwrap();
        assert!(
            manifest["capabilities"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == GET)
        );
        for mutation in ["hagency.resources.configure"] {
            assert!(
                !manifest["capabilities"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|value| value == mutation)
            );
        }
    }
}
