//! One test per TS delivery-feedback case, asserting the TS-visible notice text
//! character for character (bridge-matrix.js:6492-6572, :11045-11052).
use hagency_store::{DeliveryFeedback, DeliveryWarning, DirectTarget, MentionState, MentionTarget};
use serde_json::json;

fn text(body: &str) -> String {
    serde_json::from_value::<serde_json::Value>(json!(body))
        .map(|v| v.as_str().unwrap_or_default().to_owned())
        .unwrap_or_else(|_| body.to_owned())
}

/// `result.error` → `⚠️ Message not delivered: ${result.error}` (:6498).
#[test]
fn native_delivery_feedback_message_not_delivered() {
    let feedback = DeliveryFeedback {
        error: Some("backend refused".into()),
        warnings: vec![],
    };
    assert_eq!(
        feedback.lines(),
        vec![text("⚠️ Message not delivered: backend refused")]
    );
}

/// `target_offline` + `queued` → the queued line, with `reason` in parens
/// only when the backend sent one (:6505).
#[test]
fn native_delivery_feedback_target_offline_queued() {
    let with_reason = DeliveryFeedback {
        error: None,
        warnings: vec![DeliveryWarning::TargetOffline {
            target: "bob".into(),
            server: Some("example.test".into()),
            reason: "pane gone".into(),
            queued: true,
        }],
    };
    assert_eq!(
        with_reason.lines(),
        vec![text(
            "⚠️ @bob is offline (pane gone). Message queued; it will be delivered when the agent is online. It may be time-sensitive."
        )]
    );
    // No reason → no empty `()`: TS builds `reason` as `''` when absent (:6504).
    let without_reason = DeliveryFeedback {
        error: None,
        warnings: vec![DeliveryWarning::TargetOffline {
            target: "bob".into(),
            server: None,
            reason: String::new(),
            queued: true,
        }],
    };
    assert_eq!(
        without_reason.lines(),
        vec![text(
            "⚠️ @bob is offline. Message queued; it will be delivered when the agent is online. It may be time-sensitive."
        )]
    );
}

/// `target_offline` without `queued` → the archived-only line (:6507).
#[test]
fn native_delivery_feedback_target_offline_archived() {
    let feedback = DeliveryFeedback {
        error: None,
        warnings: vec![DeliveryWarning::TargetOffline {
            target: "bob".into(),
            server: None,
            reason: "pane gone".into(),
            queued: false,
        }],
    };
    assert_eq!(
        feedback.lines(),
        vec![text(
            "⚠️ @bob is offline (pane gone). Message archived only and was not delivered."
        )]
    );
}

/// `mentions_offline` → `@name (reason)`, `', '` joined, trailing period (:6517).
#[test]
fn native_delivery_feedback_mentions_offline() {
    let feedback = DeliveryFeedback {
        error: None,
        warnings: vec![DeliveryWarning::MentionsOffline {
            targets: vec![
                MentionTarget {
                    target: "alice".into(),
                    reason: Some("offline".into()),
                },
                MentionTarget {
                    target: "carol".into(),
                    reason: Some("pane gone".into()),
                },
            ],
        }],
    };
    assert_eq!(
        feedback.lines(),
        vec![text(
            "⚠️ Offline mentions were archived only: @alice (offline), @carol (pane gone)."
        )]
    );
}

/// `mentions_unknown` → no reason in parens, the `:6525` form.
#[test]
fn native_delivery_feedback_mentions_unknown() {
    let feedback = DeliveryFeedback {
        error: None,
        warnings: vec![DeliveryWarning::MentionsUnknown {
            targets: vec![MentionTarget {
                target: "zoe".into(),
                reason: Some("not-found".into()),
            }],
        }],
    };
    assert_eq!(
        feedback.lines(),
        vec![text(
            "⚠️ Mention targets not found in agent registry: @zoe."
        )]
    );
}

/// `mentions_not_in_group` → the `:6535` form.
#[test]
fn native_delivery_feedback_mentions_not_in_group() {
    let feedback = DeliveryFeedback {
        error: None,
        warnings: vec![DeliveryWarning::MentionsNotInGroup {
            targets: vec![MentionTarget {
                target: "dave".into(),
                reason: Some("not-in-group".into()),
            }],
        }],
    };
    assert_eq!(
        feedback.lines(),
        vec![text(
            "⚠️ Mentions not delivered because targets are not members of this group: @dave."
        )]
    );
}

/// `submitHumanMessage`'s two failure arms (:6552, :6560).
#[test]
fn native_delivery_feedback_submit_failures() {
    assert_eq!(
        DeliveryFeedback::delivery_failed_after_retry("timeout"),
        text("⚠️ Message delivery failed after retry (timeout).")
    );
    assert_eq!(
        DeliveryFeedback::delivery_failed("socket hang up"),
        text("⚠️ Message delivery failed (socket hang up).")
    );
}

/// `sendAttachmentsForMessage` (:11048), including TS's `(unknown path)`
/// substitution for a blank path.
#[test]
fn native_delivery_feedback_attachment_not_delivered() {
    assert_eq!(
        DeliveryFeedback::attachment_not_delivered("msg_1", "/work/a.pdf", "not a member"),
        text("⚠️ Attachment not delivered for msg_1: /work/a.pdf (not a member)")
    );
    assert_eq!(
        DeliveryFeedback::attachment_not_delivered("msg_2", "   ", "M_FORBIDDEN"),
        text("⚠️ Attachment not delivered for msg_2: (unknown path) (M_FORBIDDEN)")
    );
}

/// The error line comes first, warnings keep backend order, and the notice is
/// one message joined by `'\n'` (:6564-6566).
#[test]
fn native_delivery_feedback_joins_lines_in_ts_order() {
    let feedback = DeliveryFeedback {
        error: Some("backend refused".into()),
        warnings: vec![
            DeliveryWarning::TargetOffline {
                target: "bob".into(),
                server: None,
                reason: "offline".into(),
                queued: true,
            },
            DeliveryWarning::MentionsUnknown {
                targets: vec![MentionTarget {
                    target: "zoe".into(),
                    reason: None,
                }],
            },
        ],
    };
    assert_eq!(
        feedback.notice_body().unwrap(),
        text(
            "⚠️ Message not delivered: backend refused\n\
             ⚠️ @bob is offline (offline). Message queued; it will be delivered when the agent is online. It may be time-sensitive.\n\
             ⚠️ Mention targets not found in agent registry: @zoe."
        )
    );
}

/// Nothing to say → no notice: TS only calls `sendDeliveryNotice` when
/// `lines.length > 0` (:6564).
#[test]
fn native_delivery_feedback_silent_when_clean() {
    let clean = DeliveryFeedback {
        error: None,
        warnings: vec![],
    };
    assert!(clean.lines().is_empty());
    assert_eq!(clean.notice_body(), None);
}

/// The backend's own filters, ported (`backend-v2.js:16778-16830`): a
/// router-served agent is never warned about, the sender is excluded upstream,
/// `mentions_unknown` needs `!exists && !member`, and
/// `mentions_not_in_group` needs `exists && !member && != default`.
#[test]
fn native_delivery_feedback_warning_resolution_matches_backend() {
    let mentions = vec![
        MentionState {
            target: "offline_peer".into(),
            known: true,
            online: false,
            member: true,
            router_served: false,
            reason: "offline".into(),
        },
        MentionState {
            target: "router_peer".into(),
            known: true,
            online: false,
            member: true,
            router_served: true,
            reason: "offline".into(),
        },
        MentionState {
            target: "stranger".into(),
            known: false,
            online: false,
            member: false,
            router_served: false,
            reason: "offline".into(),
        },
        MentionState {
            target: "outsider".into(),
            known: true,
            online: true,
            member: false,
            router_served: false,
            reason: "offline".into(),
        },
        MentionState {
            target: "default_recipient".into(),
            known: true,
            online: true,
            member: false,
            router_served: false,
            reason: "offline".into(),
        },
    ];
    let feedback = DeliveryFeedback::warnings(None, &mentions, "default_recipient");
    assert_eq!(
        feedback.warnings,
        vec![
            DeliveryWarning::MentionsOffline {
                targets: vec![MentionTarget {
                    target: "offline_peer".into(),
                    reason: Some("offline".into()),
                }],
            },
            DeliveryWarning::MentionsUnknown {
                targets: vec![MentionTarget {
                    target: "stranger".into(),
                    reason: Some("not-found".into()),
                }],
            },
            DeliveryWarning::MentionsNotInGroup {
                targets: vec![MentionTarget {
                    target: "outsider".into(),
                    reason: Some("not-in-group".into()),
                }],
            },
        ]
    );
    // A clean router-served-only mention set says nothing at all.
    let quiet = DeliveryFeedback::warnings(
        None,
        &[MentionState {
            target: "router_peer".into(),
            known: true,
            online: false,
            member: true,
            router_served: true,
            reason: "offline".into(),
        }],
        "default_recipient",
    );
    assert!(quiet.warnings.is_empty());
    // A direct target that is offline yields the queued line; online yields none.
    let direct = DeliveryFeedback::warnings(
        Some(DirectTarget {
            target: "bob",
            server: Some("example.test"),
            reason: "offline",
            queued: true,
            online: false,
            router_served: false,
        }),
        &[],
        "default_recipient",
    );
    assert_eq!(
        direct.warnings,
        vec![DeliveryWarning::TargetOffline {
            target: "bob".into(),
            server: Some("example.test".into()),
            reason: "offline".into(),
            queued: true,
        }]
    );
    let online = DeliveryFeedback::warnings(
        Some(DirectTarget {
            target: "bob",
            server: None,
            reason: "offline",
            queued: true,
            online: true,
            router_served: false,
        }),
        &[],
        "default_recipient",
    );
    assert!(online.warnings.is_empty());
}
