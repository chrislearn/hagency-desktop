//! Shared native-controller failures. Raw provider content is never displayed.
pub(crate) fn known_code(code: &str) -> Option<&'static str> {
    const CODES: &[&str] = &[
        "owner_daemon_unavailable",
        "owner_daemon_platform_unsupported",
        "daemon_setup_required",
        "daemon_owner_root_unregistered",
        "daemon_other_owner_enrolled",
        "owner_root_not_registered",
        "owner_root_invalid",
        "owner_root_record_invalid",
        "owner_root_registry_unavailable",
        "owner_daemon_hosting_required",
        "owner_daemon_management_required",
        "daemon_command_pending",
        "request_id_reused",
        "daemon_bridge_lease_expired",
        "daemon_review_expired",
        "daemon_grant_review_expired",
        "daemon_owner_root_conflict",
        "daemon_owner_scope_mismatch",
        "daemon_logout_pending",
        "native_owner_restart_required",
        "daemon_stopping",
        "native_confirmation_expired",
        "management_scope_changed",
        "busy",
        "daemon_bridge_result_too_large",
        "daemon_bridge_journal_unavailable",
        "daemon_command_recovery_required",
        "daemon_command_receipt_expired",

    ];
    CODES.iter().copied().find(|known| *known == code)
}
pub(crate) fn message(code: &str) -> Option<&'static str> {
    let (en, zh) = match code {
        "owner_daemon_unavailable" | "owner_daemon_platform_unsupported" => (
            "The hosting service on this computer is unavailable. Start it in Agent → Hosting, then return here.",
            "这台电脑上的托管服务当前不可用。请在 Agent → 资源托管 中启动服务，然后返回这里。",
        ),
        "owner_daemon_hosting_required" | "owner_daemon_management_required" => (
            "Resume hosting in Agent → Hosting before managing this resource.",
            "请先在 Agent → 资源托管 中恢复托管，再管理此资源。",
        ),
        "daemon_setup_required" | "daemon_owner_root_unregistered" | "owner_root_not_registered" => (
            "Set up hosting for this account in Agent → Hosting. Its resources run in the background service on this computer.",
            "请在 Agent → 资源托管 中为此账号设置托管。资源会在这台电脑的后台服务中运行。",
        ),
        "daemon_other_owner_enrolled" => (
            "The hosting service on this computer serves another account. One computer user can host one account's resource pool at a time.",
            "这台电脑上的托管服务属于另一个账号。一个电脑用户同一时间只能托管一个账号的资源池。",
        ),
        "owner_root_invalid" | "owner_root_record_invalid" | "owner_root_registry_unavailable" => (
            "This computer's hosting data could not be opened safely. Check the Hagency data folders and their permissions.",
            "无法安全打开这台电脑的托管数据。请检查 Hagency 数据目录及其权限。",
        ),
        "daemon_command_pending" | "request_id_reused" => (
            "The original request is not confirmed. Its identity is saved. Recover that request from saved commands; do not create a replacement.",
            "原请求的结果尚未确认，原请求标识已保存。请从已保存的命令恢复原请求，不要新建替代请求。",
        ),
        "daemon_bridge_lease_expired" | "daemon_review_expired" | "daemon_grant_review_expired" | "native_confirmation_expired" => {
            (
                "This review expired. Close it and open the same saved request again before confirming.",
                "此确认已过期。请关闭后重新打开同一个已保存的请求，再进行确认。",
            )
        }
        "daemon_owner_root_conflict" | "daemon_owner_scope_mismatch" => (
            "The hosting service on this computer serves another resource folder or account. Set up hosting again in Agent → Hosting.",
            "这台电脑上的托管服务对应另一个资源目录或账号。请在 Agent → 资源托管 中重新设置托管。",
        ),
        "daemon_logout_pending" => (
            "Sign-out is still finishing in the owner service. Keep the service available and check the original sign-out again.",
            "资源执行服务仍在完成退出登录。请保持服务可用，并重新检查这次退出登录。",
        ),
        "native_owner_restart_required" => (
            "Resource custody changed. Close this panel and reopen it to reconnect.",
            "资源托管方式已更改。请关闭此面板后重新打开，以重新连接。",
        ),
        "daemon_stopping" => (
            "The hosting service is stopping. Wait for it to finish, then reconnect to check this request.",
            "托管服务正在停止。请等待完成后重新连接，检查此请求。",
        ),
        "management_scope_changed" => (
            "The signed-in account or server changed. Reopen this panel under the original account to review the request.",
            "登录账号或服务器已更改。请切回原账号并重新打开此面板，检查原请求。",
        ),
        "busy" => (
            "The hosting service is busy. Wait for the current operation to finish, then check the same request again.",
            "托管服务正忙。请等待当前操作完成后重新检查同一个请求。",
        ),
        "daemon_bridge_result_too_large" | "daemon_bridge_journal_unavailable" | "daemon_command_recovery_required" | "daemon_command_receipt_expired" => (
            "The service cannot confirm the original operation from its saved receipt. Keep this request and its identity; check the current resource state before attempting recovery.",
            "服务无法根据已保存的回执确认原操作。请保留此请求及其标识，先检查当前资源状态，再尝试恢复。",
        ),
        _ => return None,
    };
    Some(
        if crate::i18n::language() == crate::i18n::Language::Chinese {
            zh
        } else {
            en
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hosting_setup_codes_point_to_the_hosting_page() {
        for code in ["daemon_setup_required", "daemon_owner_root_unregistered", "owner_root_not_registered", "owner_daemon_unavailable"] {
            assert_eq!(known_code(code), Some(code));
            let text = message(code).unwrap();
            assert!(text.contains("Hosting") || text.contains("资源托管"), "{code}: {text}");
            assert!(!text.contains("Hagency Owner"));
        }
        let other = message("daemon_other_owner_enrolled").unwrap();
        assert!(other.contains("one account") || other.contains("一个账号"), "{other}");
    }

    #[test]
    fn pending_copy_requires_original_recovery_and_never_exposes_unknown_text() {
        let text = message("daemon_command_pending").unwrap();
        assert!(text.contains("original request") && text.contains("do not create a replacement"));
        assert!(message("/private/provider/home access_token=secret").is_none());
    }
}
