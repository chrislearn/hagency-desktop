//! Shared native-controller failures. Raw provider content is never displayed.
pub(crate) fn known_code(code: &str) -> Option<&'static str> {
    const CODES: &[&str] = &[
        "owner_daemon_unavailable",
        "owner_daemon_platform_unsupported",
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
            "The owner service is unavailable. Open Hagency Owner on this computer and start the service, then return here.",
            "资源执行服务当前不可用。请在这台电脑打开 Hagency Owner 并启动服务，然后返回这里。",
        ),
        "owner_daemon_hosting_required" | "owner_daemon_management_required" => (
            "Resume hosting in Hagency Owner before managing this resource.",
            "请先在 Hagency Owner 恢复托管，再管理此资源。",
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
            "This owner already has a different resource directory. Use resource handover in Hagency Owner to keep the existing resources and identities.",
            "此资源主人已有另一个资源目录。请在 Hagency Owner 使用资源迁移，保留现有资源及身份。",
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
    fn pending_copy_requires_original_recovery_and_never_exposes_unknown_text() {
        let text = message("daemon_command_pending").unwrap();
        assert!(text.contains("original request") && text.contains("do not create a replacement"));
        assert!(message("/private/provider/home access_token=secret").is_none());
    }
}
