//! The engagement-approval receipt posted to the requester room (TS parity:
//! `lib/engagement-notice.js:2-20` for the content, `lib/matrix-work-executor.js:8-12`
//! for the dispatch). The representative sends one `m.notice` into the request
//! room — a public receipt, never an execution approval or a provider DTO —
//! with the exact retained body wording, an empty `m.mentions`, and an
//! `m.in_reply_to` only when the request's source event id is a real Matrix
//! event id (`$…`).

use serde_json::{Value, json};

/// The retained notice content, byte-for-byte in the body lines
/// (`engagementApprovalContent`, lib/engagement-notice.js:2-20).
///
/// - `configuration` is `serving.framework · serving.model · serving.reasoning`,
///   joined with ` · ` over the present fields only, and the `Serving:` line is
///   omitted entirely when nothing joined (TS `.filter(Boolean).join(' · ')`).
/// - The closing instruction names the target project room when the request was
///   filed with a request context (TS `engagement.requestContext`), else this
///   room.
/// - `m.mentions` is always the empty allowlist; `m.relates_to` is set only
///   when `source_event_id` starts with `$` (TS line 15-18).
// Not called yet: the request-room approval notice is still unsent natively.
#[allow(dead_code, clippy::too_many_arguments)]
pub fn engagement_approval_content(
    engagement_id: &str,
    role: &str,
    allocated_tokens: u64,
    project_room_id: &str,
    request_context: bool,
    source_event_id: Option<&str>,
    serving: [Option<&str>; 3],
    mxid: &str,
) -> Value {
    let configuration = serving
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    let mut body =
        format!("已批准 / Approved {role} for {allocated_tokens} tokens.\nAgent: {mxid}\n");
    if !configuration.is_empty() {
        body.push_str(&format!("Serving: {configuration}\n"));
    }
    body.push_str(&format!("Request: {engagement_id}\n"));
    if request_context {
        body.push_str(&format!(
            "目标项目 / Target project: {project_room_id}\n请在目标项目房间 @ 该 Agent 开始任务。Mention this agent in the target project room to begin work."
        ));
    } else {
        body.push_str(
            "请在此房间 @ 该 Agent 开始任务。Mention this agent in this room to begin work.",
        );
    }
    let mut content = json!({
        "msgtype": "m.notice",
        "body": body,
        "m.mentions": { "user_ids": [] },
    });
    if let Some(source_event_id) = source_event_id
        && source_event_id.starts_with('$')
    {
        content["m.relates_to"] = json!({ "m.in_reply_to": { "event_id": source_event_id } });
    }
    content
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TS parity, lib/engagement-notice.js:5-13: the body lines for a request
    /// with context and a full serving configuration.
    #[test]
    fn engagement_notice_body_with_context_and_serving() {
        let content = engagement_approval_content(
            "en_abc",
            "reviewer",
            250_000,
            "!project:example.test",
            true,
            Some("$requestevent"),
            [Some("codex"), Some("gpt-5"), Some("high")],
            "@hf_agent:example.test",
        );
        assert_eq!(content["msgtype"], "m.notice");
        assert_eq!(
            content["body"].as_str().unwrap(),
            "已批准 / Approved reviewer for 250000 tokens.\n\
             Agent: @hf_agent:example.test\n\
             Serving: codex · gpt-5 · high\n\
             Request: en_abc\n\
             目标项目 / Target project: !project:example.test\n\
             请在目标项目房间 @ 该 Agent 开始任务。Mention this agent in the target project room to begin work."
        );
        assert_eq!(content["m.mentions"], json!({ "user_ids": [] }));
        // TS lines 15-18: a `$`-prefixed source event threads the receipt.
        assert_eq!(
            content["m.relates_to"],
            json!({ "m.in_reply_to": { "event_id": "$requestevent" } })
        );
    }

    /// TS parity: partial serving fields join with ` · `; without a request
    /// context the closing instruction names this room; a non-`$` request id
    /// adds no relation.
    #[test]
    fn engagement_notice_body_without_context_and_partial_serving() {
        let content = engagement_approval_content(
            "en_def",
            "builder",
            1000,
            "!project:example.test",
            false,
            Some("req-not-an-event"),
            [Some("claude"), None, None],
            "@hf_other:example.test",
        );
        assert_eq!(
            content["body"].as_str().unwrap(),
            "已批准 / Approved builder for 1000 tokens.\n\
             Agent: @hf_other:example.test\n\
             Serving: claude\n\
             Request: en_def\n\
             请在此房间 @ 该 Agent 开始任务。Mention this agent in this room to begin work."
        );
        assert!(content.get("m.relates_to").is_none());
    }

    /// TS parity, line 9: an empty configuration omits the Serving line whole.
    #[test]
    fn engagement_notice_body_without_serving() {
        let content = engagement_approval_content(
            "en_ghi",
            "reviewer",
            5,
            "!p:example.test",
            false,
            None,
            [None, None, None],
            "@a:b",
        );
        let body = content["body"].as_str().unwrap();
        assert!(!body.contains("Serving:"));
        assert_eq!(
            body,
            "已批准 / Approved reviewer for 5 tokens.\nAgent: @a:b\nRequest: en_ghi\n请在此房间 @ 该 Agent 开始任务。Mention this agent in this room to begin work."
        );
        assert!(content.get("m.relates_to").is_none());
    }
}
