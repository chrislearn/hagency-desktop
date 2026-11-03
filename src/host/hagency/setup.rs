//! Closed managed-project setup calls. The native transport owns command IDs,
//! private creation custody and recovery; Splash cannot supply a Matrix payload.
use super::{parse, value};
use ::hagency::native_owner::{
    AssignAdministrator, BeginSetup, ManagementPermission,
    RebaseSetup, SetupCandidates,
};
use ::hagency::daemon::bridge::{ManagementClient as NativeManagement, ProjectSetupTransport};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

const JSON_SAFE_MAX: u64 = 9_007_199_254_740_991;

#[derive(Debug)]
pub(super) enum SetupCall {
    Progress(String),
    Begin(BeginSetup),
    Continue(String, SetupCandidates),
    Resume(String),
    Cancel {
        project: String,
        setup: String,
        revision: u64,
    },
    Rebase(RebaseSetup),
    Retry(String),
    Acknowledge(String),
    Assign(AssignAdministrator),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Project {
    project_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Continue {
    project_id: String,
    space_id: Option<String>,
    room_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Cancel {
    project_id: String,
    setup_id: String,
    expected_revision: u64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Command {
    command_id: String,
}

fn require(valid: bool) -> Result<(), String> {
    if valid {
        Ok(())
    } else {
        Err("invalid_management_arguments".into())
    }
}
// These are input-shape checks before lazy owner initialization. The native
// transport independently validates identity, authority, CAS and journal state.
fn key(id: &str) -> Result<(), String> {
    require(
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
    )
}
fn revision(value: u64, allow_zero: bool) -> Result<(), String> {
    require(value <= JSON_SAFE_MAX && (allow_zero || value > 0))
}
fn mxid(value: &str) -> Result<(), String> {
    require(
        value.len() <= 255
            && value.split_once(':').is_some_and(|(local, server)| {
                local.starts_with('@') && local.len() > 1 && !server.is_empty()
            })
            && !value.chars().any(|c| c.is_whitespace() || c.is_control()),
    )
}
fn room_id(value: &str) -> Result<(), String> {
    require(
        value.starts_with('!')
            && value.len() <= 255
            && !value
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || ['/', '?', '#', '\\'].contains(&c)),
    )
}

/// Services are exact names, not a prefix grant. Recovery adds no mutation
/// permission: native checks the original saved command's permission as well.
pub(super) fn permissions(service: &str) -> &'static [ManagementPermission] {
    use ManagementPermission::*;
    match service {
        "hagency.projects.setup.progress"
        | "hagency.projects.setup.retry"
        | "hagency.projects.setup.acknowledge" => &[ReadProjectSetup],
        "hagency.projects.setup.begin"
        | "hagency.projects.setup.resume"
        | "hagency.projects.setup.cancel"
        | "hagency.projects.setup.rebase" => &[ReadProjectSetup, ManageProjectSetup],
        "hagency.projects.setup.continue" => {
            &[ReadProjectSetup, ManageProjectSetup, CreateProjectRooms]
        }
        "hagency.projects.administrator.assign" => &[ReadProjectSetup, AssignProjectAdministrators],
        _ => &[],
    }
}

pub(super) fn parse_call(service: &str, args: Value) -> Result<Option<SetupCall>, String> {
    Ok(Some(match service {
        "hagency.projects.setup.progress" | "hagency.projects.setup.resume" => {
            let input: Project = parse(args)?;
            key(&input.project_id)?;
            if service == "hagency.projects.setup.progress" {
                SetupCall::Progress(input.project_id)
            } else {
                SetupCall::Resume(input.project_id)
            }
        }
        "hagency.projects.setup.begin" => {
            let input: BeginSetup = parse(args)?;
            key(&input.project_id)?;
            key(&input.allocation_id)?;
            for n in [
                input.expected_project_revision,
                input.expected_allocation_revision,
                input.expected_authority_revision,
            ] {
                revision(n, false)?;
            }
            SetupCall::Begin(input)
        }
        "hagency.projects.setup.continue" => {
            let input: Continue = parse(args)?;
            key(&input.project_id)?;
            for id in [&input.space_id, &input.room_id].into_iter().flatten() {
                room_id(id)?;
            }
            require(input.space_id.is_none() || input.space_id != input.room_id)?;
            SetupCall::Continue(
                input.project_id,
                SetupCandidates {
                    space_id: input.space_id,
                    room_id: input.room_id,
                },
            )
        }
        "hagency.projects.setup.cancel" => {
            let input: Cancel = parse(args)?;
            key(&input.project_id)?;
            key(&input.setup_id)?;
            revision(input.expected_revision, false)?;
            SetupCall::Cancel {
                project: input.project_id,
                setup: input.setup_id,
                revision: input.expected_revision,
            }
        }
        "hagency.projects.setup.rebase" => {
            let input: RebaseSetup = parse(args)?;
            for id in [&input.project_id, &input.setup_id, &input.allocation_id] {
                key(id)?;
            }
            for n in [
                input.expected_revision,
                input.expected_project_revision,
                input.expected_allocation_revision,
                input.expected_authority_revision,
            ] {
                revision(n, false)?;
            }
            SetupCall::Rebase(input)
        }
        "hagency.projects.setup.retry" | "hagency.projects.setup.acknowledge" => {
            let input: Command = parse(args)?;
            key(&input.command_id)?;
            if service == "hagency.projects.setup.retry" {
                SetupCall::Retry(input.command_id)
            } else {
                SetupCall::Acknowledge(input.command_id)
            }
        }
        "hagency.projects.administrator.assign" => {
            let input: AssignAdministrator = parse(args)?;
            key(&input.project_id)?;
            revision(input.expected_authority_revision, true)?;
            mxid(&input.administrator_mxid)?;
            SetupCall::Assign(input)
        }
        _ => return Ok(None),
    }))
}

impl SetupCall {
    pub(super) async fn execute(self, manager: Arc<NativeManagement>) -> Result<Value, String> {
        let transport = ProjectSetupTransport::new(manager);
        let result = match self {
            Self::Progress(project) => transport.progress(project).await,
            Self::Begin(input) => transport.begin_create(input).await,
            Self::Continue(project, candidates) => {
                transport.continue_setup(project, candidates).await
            }
            Self::Resume(project) => transport.resume_begin(project).await,
            Self::Cancel {
                project,
                setup,
                revision,
            } => transport.cancel(project, setup, revision).await,
            Self::Rebase(input) => transport.rebase(input).await,
            Self::Retry(command) => transport.retry_command(command).await,
            Self::Acknowledge(command) => transport.acknowledge_failure(command).await,
            Self::Assign(input) => transport.assign_administrator(input).await,
        };
        result.map_err(|error| error.code).and_then(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn setup_services_require_exact_reviewed_lease_and_ship_only_used_grants() {
        let authority = rinx_miniapp_core::SessionAuthority::default();
        let identity = crate::miniapps::InstanceId {
            app: octosense_app_contract::hagency::APP_ID.into(),
            account: "@manager:test".into(),
            room: None,
            generation: 1,
        };
        let issue = |services: &[&str]| {
            authority.issue(
                identity.clone(),
                services.iter().map(|s| s.to_string()).collect(),
                Default::default(),
                std::time::Instant::now() + std::time::Duration::from_secs(60),
            )
        };
        let readonly = issue(&[
            "hagency.projects.setup.progress",
            "hagency.projects.setup.retry",
        ]);
        assert_eq!(
            super::super::permissions(&readonly),
            vec![ManagementPermission::ReadProjectSetup]
        );
        assert!(
            readonly
                .authorize("@manager:test", "hagency.projects.setup.continue", None)
                .is_err()
        );
        let assignment = issue(&["hagency.projects.administrator.assign"]);
        assert_eq!(
            super::super::permissions(&assignment),
            vec![
                ManagementPermission::ReadProjectSetup,
                ManagementPermission::AssignProjectAdministrators
            ]
        );
        assert!(
            assignment
                .authorize("@manager:test", "hagency.projects.setup.begin", None)
                .is_err()
        );
        let shipped: Value =
            serde_json::from_str(include_str!("../../../apps/hagency/bundle/manifest.json"))
                .unwrap();
        let expected = std::collections::BTreeSet::from([
            "hagency.projects.setup.progress",
            "hagency.projects.setup.begin",
            "hagency.projects.setup.continue",
            "hagency.projects.setup.cancel",
            "hagency.projects.setup.rebase",
            "hagency.projects.setup.retry",
            "hagency.projects.setup.acknowledge",
            "hagency.projects.administrator.assign",
        ]);
        let actual = shipped["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .filter(|service| !permissions(service).is_empty())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(actual, expected);
        for service in octosense_app_contract::hagency::SERVICES {
            if !permissions(service).is_empty() {
                assert!(octosense_app_contract::hagency::words(service).is_some());
            }
        }
    }

    #[test]
    fn begin_cannot_supply_creation_identity_marker_or_payload() {
        let input = json!({"projectId":"project_1","allocationId":"allocation_1","expectedProjectRevision":2,
            "expectedAllocationRevision":4,"expectedAuthorityRevision":1});
        assert!(matches!(
            parse_call("hagency.projects.setup.begin", input.clone()),
            Ok(Some(SetupCall::Begin(_)))
        ));
        for field in [
            "commandId",
            "actor",
            "managerMxid",
            "token",
            "url",
            "mode",
            "name",
            "marker",
            "initialState",
            "allowCreateRoom",
            "setupId",
            "currentBinding",
        ] {
            let mut injected = input.clone();
            injected[field] = json!("injected");
            assert!(
                parse_call("hagency.projects.setup.begin", injected).is_err(),
                "{field}"
            );
        }
        for n in [
            json!(-1),
            json!(0),
            json!(1.5),
            json!("2"),
            json!(JSON_SAFE_MAX + 1),
        ] {
            let mut invalid = input.clone();
            invalid["expectedAllocationRevision"] = n;
            assert!(parse_call("hagency.projects.setup.begin", invalid).is_err());
        }
    }

    #[test]
    fn recovery_has_only_original_ids_and_room_candidates_never_creation_payloads() {
        for service in [
            "hagency.projects.setup.retry",
            "hagency.projects.setup.acknowledge",
        ] {
            assert!(parse_call(service, json!({"commandId":"original_1"})).is_ok());
            for field in ["operation", "input", "reply", "authority", "projectId"] {
                let mut input = json!({"commandId":"original_1"});
                input[field] = json!({});
                assert!(parse_call(service, input).is_err());
            }
        }
        let candidates =
            json!({"projectId":"project_1","spaceId":"!space:test","roomId":"!room:test"});
        assert!(matches!(
            parse_call("hagency.projects.setup.continue", candidates),
            Ok(Some(SetupCall::Continue(_, _)))
        ));
        for input in [
            json!({"projectId":"project_1","spaceId":"!same:test","roomId":"!same:test"}),
            json!({"projectId":"project_1","spaceId":"https://example.test/room"}),
            json!({"projectId":"project_1","roomId":"!room:test/path"}),
            json!({"projectId":"project_1","createRoom":true}),
            json!({"projectId":"project_1","allowCreateRoom":true}),
            json!({"projectId":"project_1","marker":{"setupId":"injected"}}),
        ] {
            assert!(parse_call("hagency.projects.setup.continue", input).is_err());
        }
    }

    #[test]
    fn setup_capabilities_do_not_alias_server_roles_or_recovery_authority() {
        use ManagementPermission::*;
        for service in [
            "hagency.projects.setup.progress",
            "hagency.projects.setup.retry",
            "hagency.projects.setup.acknowledge",
        ] {
            assert_eq!(permissions(service), &[ReadProjectSetup]);
        }
        assert_eq!(
            permissions("hagency.projects.setup.continue"),
            &[ReadProjectSetup, ManageProjectSetup, CreateProjectRooms]
        );
        assert_eq!(
            permissions("hagency.projects.administrator.assign"),
            &[ReadProjectSetup, AssignProjectAdministrators]
        );
        for service in [
            "hagency.projects.setup.begin",
            "hagency.projects.setup.cancel",
            "hagency.projects.setup.rebase",
            "hagency.projects.setup.resume",
        ] {
            assert_eq!(
                permissions(service),
                &[ReadProjectSetup, ManageProjectSetup]
            );
        }
        for service in [
            "hagency.projects.decide",
            "hagency.inbox.decide",
            "hagency.intent.retry",
            "hagency.projects.setup",
            "hagency.projects.setup.confirm",
            "hagency.projects.setup.command",
        ] {
            assert!(permissions(service).is_empty());
            assert!(parse_call(service, json!({})).unwrap().is_none());
        }
    }

    #[test]
    fn all_routes_reject_unknown_fields_and_unsafe_identifiers_before_initialization() {
        let routes = [
            (
                "hagency.projects.setup.progress",
                json!({"projectId":"project_1"}),
            ),
            (
                "hagency.projects.setup.resume",
                json!({"projectId":"project_1"}),
            ),
            (
                "hagency.projects.setup.continue",
                json!({"projectId":"project_1"}),
            ),
            (
                "hagency.projects.setup.cancel",
                json!({"projectId":"project_1","setupId":"setup_1","expectedRevision":3}),
            ),
            (
                "hagency.projects.setup.rebase",
                json!({"projectId":"project_1","setupId":"setup_1","expectedRevision":3,"allocationId":"allocation_2","expectedProjectRevision":2,"expectedAllocationRevision":4,"expectedAuthorityRevision":2}),
            ),
            (
                "hagency.projects.administrator.assign",
                json!({"projectId":"project_1","expectedAuthorityRevision":0,"administratorMxid":"@admin:test"}),
            ),
        ];
        for (service, input) in routes {
            assert!(
                parse_call(service, input.clone()).unwrap().is_some(),
                "{service}"
            );
            let mut extra = input.clone();
            extra["token"] = json!("not permitted");
            assert!(parse_call(service, extra).is_err(), "{service}");
            for id in ["", "../project", "project?token=x", &"a".repeat(129)] {
                let mut invalid = input.clone();
                invalid["projectId"] = json!(id);
                assert!(parse_call(service, invalid).is_err(), "{service}");
            }
            for invalid in [
                json!(null),
                json!([]),
                json!({"oversized":"x".repeat(16*1024)}),
            ] {
                assert!(parse_call(service, invalid).is_err(), "{service}");
            }
        }
    }
}
