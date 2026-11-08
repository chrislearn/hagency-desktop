//! Join backfill (board #79; TS `bridge-matrix.js:2940-2976` `pendingJoinBackfill`).
//!
//! When a member (bot, representative or agent) is admitted to a room, the
//! discussion that happened between its invite and its join has already fallen
//! off the sync window. This selector names exactly which events that window
//! holds, fail-closed: without a provable invite of OURS in the page, NOTHING is
//! routed — replaying a command nobody just issued is worse than missing it.
//!
//! `/messages?dir=b` is newest-first; every index below is in TIMELINE order,
//! exactly as the TS selector computes it (`[...chunk].reverse()`).
use serde_json::Value;

/// The TS boundary word: `'invite..join'` when the page holds our join, `'invite..end'`
/// when everything in the page predates it, `'unproven'` when no invite of ours is
/// provable, `'no-input'` when the page or the identity is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boundary {
    InviteToJoin,
    InviteToEnd,
    Unproven,
    NoInput,
}

impl Boundary {
    /// The TS literal (`:2976` `boundary`), for logs and tests.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InviteToJoin => "invite..join",
            Self::InviteToEnd => "invite..end",
            Self::Unproven => "unproven",
            Self::NoInput => "no-input",
        }
    }
}

/// The events the invite..join window holds, oldest first (timeline order).
#[derive(Debug, Clone, PartialEq)]
pub struct BackfillWindow {
    /// The `event_id`s TS routed. Events without an id, non-messages, our own
    /// sends and already-seen ids are filtered inside the window (:2961-2968).
    pub events: Vec<String>,
    pub boundary: Boundary,
}

/// The LAST invite of ours, and the join that follows it
/// (bridge-matrix.js:2949-2958). A room we were invited to, left, and re-invited
/// to carries several; anchoring on an older one replays the previous
/// membership's commands.
///
/// `chunk` is the raw `/messages?dir=b` response segment (newest-first) and
/// `user_id` is the member being backfilled (TS `botUserId`).
pub fn pending_join_backfill(
    chunk: Option<&Value>,
    user_id: &str,
    seen: Option<&std::collections::BTreeSet<String>>,
) -> BackfillWindow {
    let none = BackfillWindow {
        events: Vec::new(),
        boundary: Boundary::NoInput,
    };
    // `!Array.isArray(chunk) || !botUserId` -> no-input (:2941).
    let Some(chunk) = chunk.and_then(Value::as_array) else {
        return none;
    };
    if user_id.is_empty() {
        return none;
    }
    // /messages?dir=b is newest-first; work in timeline order (:2942).
    let timeline: Vec<&Value> = chunk.iter().rev().collect();
    let mut invite_idx: Option<usize> = None;
    let mut join_idx: Option<usize> = None;
    for (index, event) in timeline.iter().enumerate() {
        if event.get("type").and_then(Value::as_str) != Some("m.room.member")
            || event.get("state_key").and_then(Value::as_str) != Some(user_id)
        {
            continue;
        }
        match event.pointer("/content/membership").and_then(Value::as_str) {
            Some("invite") => {
                invite_idx = Some(index);
                join_idx = None;
            }
            Some("join") if invite_idx.is_some() && join_idx.is_none() => {
                join_idx = Some(index);
            }
            _ => {}
        }
    }
    // No provable invite of ours: nothing is routed (:2959).
    let Some(invite_idx) = invite_idx else {
        return BackfillWindow {
            events: Vec::new(),
            boundary: Boundary::Unproven,
        };
    };
    // No join in the page means we joined after everything it contains; the
    // window is the rest of the page (:2961-2963).
    let end = join_idx.unwrap_or(timeline.len());
    let mut events = Vec::new();
    for event in timeline.iter().take(end).skip(invite_idx + 1) {
        if event.get("type").and_then(Value::as_str) != Some("m.room.message") {
            continue;
        }
        let Some(id) = event.get("event_id").and_then(Value::as_str) else {
            continue;
        };
        // Our own messages would loop back into us (:2966).
        if event.get("sender").and_then(Value::as_str) == Some(user_id) {
            continue;
        }
        if let Some(seen) = seen
            && seen.contains(id)
        {
            continue;
        }
        events.push(id.to_owned());
    }
    BackfillWindow {
        events,
        boundary: if join_idx.is_some() {
            Boundary::InviteToJoin
        } else {
            Boundary::InviteToEnd
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const BOT: &str = "@bot:matrix.test";

    fn member(kind: &str, ts: u64) -> Value {
        json!({"type":"m.room.member","sender":"@other:matrix.test","origin_server_ts":ts,
            "state_key":BOT,"content":{"membership":kind}})
    }
    fn member_of(kind: &str, ts: u64, who: &str) -> Value {
        json!({"type":"m.room.member","sender":"@other:matrix.test","origin_server_ts":ts,
            "state_key":who,"content":{"membership":kind}})
    }
    fn msg(id: &str, body: &str) -> Value {
        msg_from(id, body, "@lin:matrix.test", 1100)
    }
    fn msg_from(id: &str, body: &str, sender: &str, ts: u64) -> Value {
        json!({"type":"m.room.message","event_id":id,"sender":sender,
            "origin_server_ts":ts,"content":{"msgtype":"m.text","body":body}})
    }

    /// TS `join-backfill.test.js:90`: an older-in-timeline command with a NEWER
    /// timestamp is not admitted (position decides, not the clock).
    #[test]
    fn ts_oracle_backfill_older_in_timeline_with_newer_ts() {
        // /messages?dir=b newest-first: the join (1200) comes FIRST in the chunk.
        let chunk = json!([
            member("join", 1200),
            msg_from(
                "$real",
                "!request architect 300000 20000",
                "@lin:matrix.test",
                1100
            ),
            member("invite", 1000)
        ]);
        let window = pending_join_backfill(Some(&chunk), BOT, None);
        assert_eq!(window.events, vec!["$real".to_owned()]);
        assert_eq!(window.boundary, Boundary::InviteToJoin);
    }

    /// TS `:106`: commands sharing a millisecond keep timeline order, not fetch
    /// order — the sort is positional and stable exactly because there is none.
    #[test]
    fn ts_oracle_backfill_same_millisecond_keeps_timeline_order() {
        let chunk = json!([
            member("join", 1200),
            msg("$cancel", "!cancel"),
            msg("$request", "!request architect 1 1"),
            member("invite", 1000)
        ]);
        let window = pending_join_backfill(Some(&chunk), BOT, None);
        assert_eq!(
            window.events,
            vec!["$request".to_owned(), "$cancel".to_owned()]
        );
    }

    /// TS `:123`: a command sent between the invite and the join is delivered.
    #[test]
    fn ts_oracle_backfill_between_invite_and_join() {
        let chunk = json!([
            member("join", 1200),
            msg("$b", "!request a b"),
            member("invite", 1000)
        ]);
        assert_eq!(
            pending_join_backfill(Some(&chunk), BOT, None).events,
            vec!["$b".to_owned()]
        );
    }

    /// TS `:132`: a command that arrives after the join is left to sync — not
    /// this function's window, or it gets two chances to double-handle.
    #[test]
    fn ts_oracle_backfill_after_join_left_to_sync() {
        let chunk = json!([
            msg_from("$after", "b", "@lin:matrix.test", 1300),
            member("join", 1200),
            msg("$before", "a"),
            member("invite", 1000)
        ]);
        assert_eq!(
            pending_join_backfill(Some(&chunk), BOT, None).events,
            vec!["$before".to_owned()]
        );
    }

    /// TS `:141`: with no join yet in the page, everything after the invite is
    /// the window (`boundary: 'invite..end'`).
    #[test]
    fn ts_oracle_backfill_no_join_is_invite_to_end() {
        let chunk = json!([msg("$b", "b"), msg("$a", "a"), member("invite", 1000)]);
        let window = pending_join_backfill(Some(&chunk), BOT, None);
        assert_eq!(window.events, vec!["$a".to_owned(), "$b".to_owned()]);
        assert_eq!(window.boundary, Boundary::InviteToEnd);
    }

    /// TS `:150`: with no provable invite in the page, NOTHING is routed. The
    /// bounded-window fallback that used to sit here was fail-open by
    /// construction; commands are executable, so missing beats replaying.
    #[test]
    fn ts_oracle_backfill_no_invite_routs_nothing() {
        let chunk = json!([
            msg("$older", "!approve"),
            msg("$old", "!request architect 999999 99999")
        ]);
        let window = pending_join_backfill(Some(&chunk), BOT, None);
        assert!(window.events.is_empty());
        assert_eq!(window.boundary, Boundary::Unproven);
    }

    /// TS `:163`: history before an EARLIER membership cycle is not replayed on
    /// re-invite — the CURRENT invite is the only valid anchor.
    #[test]
    fn ts_oracle_backfill_earlier_cycle_not_replayed() {
        let chunk = json!([
            member("join", 1600),
            msg_from(
                "$thisCycle",
                "!request architect 300000 20000",
                "@lin:matrix.test",
                1500
            ),
            member("invite", 1400),
            member("leave", 1300),
            member("join", 1200),
            msg_from(
                "$oldCycle",
                "!request architect 999999 99999",
                "@lin:matrix.test",
                1100
            ),
            member("invite", 1000)
        ]);
        assert_eq!(
            pending_join_backfill(Some(&chunk), BOT, None).events,
            vec!["$thisCycle".to_owned()]
        );
    }

    /// TS `:180`: another user's invite is not mistaken for ours (the colleague
    /// invited AFTER the bot is the ordinary sequence, so this is load-bearing).
    #[test]
    fn ts_oracle_backfill_foreign_invite_not_ours() {
        let chunk = json!([
            member("join", 1200),
            member_of("invite", 1100, "@someone:matrix.test"),
            msg("$ours", "!request architect 1 1"),
            member("invite", 1000)
        ]);
        assert_eq!(
            pending_join_backfill(Some(&chunk), BOT, None).events,
            vec!["$ours".to_owned()]
        );
    }

    /// TS `:197`: a JOIN membership is not an invite.
    #[test]
    fn ts_oracle_backfill_join_is_not_invite() {
        let chunk = json!([msg("$m", "!request architect 1 1"), member("join", 1000)]);
        assert_eq!(
            pending_join_backfill(Some(&chunk), BOT, None).boundary,
            Boundary::Unproven
        );
    }

    /// TS `:204`: our own messages are never fed back to us.
    #[test]
    fn ts_oracle_backfill_own_messages_excluded() {
        let chunk = json!([
            member("join", 1200),
            msg_from(
                "$theirs",
                "!request architect 1 1",
                "@lin:matrix.test",
                1100
            ),
            msg_from("$mine", "!help", BOT, 1100),
            member("invite", 1000)
        ]);
        assert_eq!(
            pending_join_backfill(Some(&chunk), BOT, None).events,
            vec!["$theirs".to_owned()]
        );
    }

    /// TS `:211`: already-seen events are skipped, so sync and backfill cannot
    /// double-handle one event.
    #[test]
    fn ts_oracle_backfill_seen_skipped() {
        let chunk = json!([
            member("join", 1200),
            msg("$fresh", "b"),
            msg("$dup", "a"),
            member("invite", 1000)
        ]);
        let seen = std::collections::BTreeSet::from(["$dup".to_owned()]);
        assert_eq!(
            pending_join_backfill(Some(&chunk), BOT, Some(&seen)).events,
            vec!["$fresh".to_owned()]
        );
    }

    /// TS `:219`: non-message events and events without an id are ignored.
    #[test]
    fn ts_oracle_backfill_non_message_and_no_id_ignored() {
        let chunk = json!([
            member("join", 1200),
            json!({"type":"m.room.message","content":{"body":"no id"}}),
            json!({"type":"m.room.topic","event_id":"$t","content":{"topic":"x"}}),
            msg("$ok", "!request architect 1 1"),
            member("invite", 1000)
        ]);
        assert_eq!(
            pending_join_backfill(Some(&chunk), BOT, None).events,
            vec!["$ok".to_owned()]
        );
    }

    /// TS `:230`: a missing or malformed page yields nothing rather than
    /// throwing, and with no identity there is no boundary to find.
    #[test]
    fn ts_oracle_backfill_malformed_page_yields_nothing() {
        for bad in [None, Some(json!("nope")), Some(json!({})), Some(json!(42))] {
            assert!(
                pending_join_backfill(bad.as_ref(), BOT, None)
                    .events
                    .is_empty()
            );
        }
        assert_eq!(
            pending_join_backfill(Some(&json!([member("invite", 1000)])), "", None).boundary,
            Boundary::NoInput
        );
    }
}
