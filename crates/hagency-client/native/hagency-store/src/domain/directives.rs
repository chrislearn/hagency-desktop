//! `/thread` session directives (TS parity: `backend-v2.js:2131-2310`).
//!
//! An operator addresses an agent with `/thread model <name|default>` or
//! `/thread mode <plan|auto>` in a room. It is a directive, never chat input:
//! it updates the thread session's `model_override`/`mode_override` and is
//! answered in-thread with a confirmation notice. A non-operator directive is
//! consumed with a refusal notice, not applied; a malformed one is consumed
//! with a usage notice. Delivery never fails because of a directive
//! (`backend-v2.js:2261-2266`): the bridge consumes it and answers instead of
//! posting a delivery-failure warning into the room.
//!
//! This module is the PURE half — the parser and the confirmation text — so the
//! TS-visible words and the decision logic are asserted directly, exactly as the
//! retained `parseThreadSessionDirective` / `threadSessionDirectiveConfirmation`
//! are. The store applies the resulting override and the host answers through
//! the command-notice path; neither is here.

use serde::{Deserialize, Serialize};

use super::DomainRepository;
use crate::Error;
use hagency_core::project::identifier;
use rusqlite::{OptionalExtension, TransactionBehavior, params};

/// The retained usage line (`backend-v2.js:2143`), verbatim.
pub const THREAD_DIRECTIVE_USAGE: &str =
    "usage: /thread model <name|default> | /thread mode <plan|auto>";

/// The non-operator refusal (`backend-v2.js:2266`), verbatim.
pub const THREAD_DIRECTIVE_OPERATOR_REFUSAL: &str =
    "/thread directives require operator trust; the message was not delivered as chat.";

/// A malformed model's refusal (`backend-v2.js:2152`), verbatim.
pub const THREAD_DIRECTIVE_MODEL_REFUSAL: &str =
    "model must be a plain model name or alias (max 64 chars)";

/// A session's write mode override (`router/src/store.ts:82`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThreadMode {
    Plan,
    Auto,
}

/// What a valid directive asks to set. `None` is the `default|reset|clear`
/// verb, which clears the override (`backend-v2.js:2150,2158`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadDirective {
    Model(Option<String>),
    Mode(Option<ThreadMode>),
}

/// Whether a message body is a `/thread` directive at all (TS `/^\/thread\b/i`,
/// after stripping the address). `parse` calls this; it is also the intake gate
/// that keeps a directive from ever becoming chat input.
pub fn is_directive(body: &str) -> bool {
    let body = strip_address(body);
    let Some(rest) = body.strip_prefix("/thread") else {
        return false;
    };
    rest.is_empty() || rest.starts_with(char::is_whitespace)
}

/// The parse outcome. `None` means the body is not a `/thread` directive at
/// all; `Err(body)` is a malformed directive whose `body` is the in-thread
/// notice (usage or model refusal), never a delivery failure.
pub fn parse(body: &str) -> Option<Result<ThreadDirective, String>> {
    let body = strip_address(body);
    if !is_directive(&body) {
        return None;
    }
    // `/thread\s+(\S+)(?:\s+(\S+))?\s*$` — the whole trimmed body, two tokens max.
    let rest = body["/thread".len()..].trim();
    if rest.is_empty() {
        return Some(Err(THREAD_DIRECTIVE_USAGE.into()));
    }
    let mut tokens = rest.split_whitespace();
    let key = tokens.next().unwrap_or_default().to_ascii_lowercase();
    let value = tokens.next();
    if tokens.next().is_some() {
        // A third token means the line did not match the two-token shape.
        return Some(Err(THREAD_DIRECTIVE_USAGE.into()));
    }
    match key.as_str() {
        "model" => Some(match value {
            None => Err(THREAD_DIRECTIVE_USAGE.into()),
            Some(v) if is_clear_word(v) => Ok(ThreadDirective::Model(None)),
            Some(v) if is_model(v) => Ok(ThreadDirective::Model(Some(v.to_owned()))),
            Some(_) => Err(THREAD_DIRECTIVE_MODEL_REFUSAL.into()),
        }),
        "mode" => Some(match value {
            None => Err(THREAD_DIRECTIVE_USAGE.into()),
            Some(v) if is_clear_word(v) => Ok(ThreadDirective::Mode(None)),
            Some(v) if v.eq_ignore_ascii_case("plan") => {
                Ok(ThreadDirective::Mode(Some(ThreadMode::Plan)))
            }
            Some(v) if v.eq_ignore_ascii_case("auto") => {
                Ok(ThreadDirective::Mode(Some(ThreadMode::Auto)))
            }
            Some(_) => Err(THREAD_DIRECTIVE_USAGE.into()),
        }),
        _ => Some(Err(THREAD_DIRECTIVE_USAGE.into())),
    }
}

/// The confirmation notice (`threadSessionDirectiveConfirmation`,
/// `backend-v2.js:2161-2169`), verbatim: the resolved model and mode, plus the
/// write warning only when mode is `auto`.
pub fn confirmation(model_override: Option<&str>, mode_override: Option<ThreadMode>) -> String {
    let model = model_override.map_or("model=default".to_owned(), |m| format!("model={m}"));
    let mode = match mode_override {
        Some(ThreadMode::Auto) => "mode=auto".to_owned(),
        Some(ThreadMode::Plan) => "mode=plan".to_owned(),
        None => "mode=default (read-only)".to_owned(),
    };
    let write_warning = if mode_override == Some(ThreadMode::Auto) {
        " Runners in this thread may now write to the agent workspace (writes stay serialized by workspace lease)."
    } else {
        ""
    };
    format!("Thread session updated: {model}, {mode}.{write_warning}")
}

/// `default|reset|clear` clears the override (`backend-v2.js:2150,2158`).
fn is_clear_word(value: &str) -> bool {
    value.eq_ignore_ascii_case("default")
        || value.eq_ignore_ascii_case("reset")
        || value.eq_ignore_ascii_case("clear")
}

/// `THREAD_DIRECTIVE_MODEL_PATTERN` (`backend-v2.js:2129`):
/// `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$` — a plain model name/alias ≤64 chars.
fn is_model(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() || value.len() > 64 {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Strip mention pills and a leading `@name` address so the directive can lead
/// the visible text (`backend-v2.js:2133-2137`): markdown links first, then any
/// leading `@local[:server]` tokens.
fn strip_address(body: &str) -> String {
    let mut stripped = body;
    // `\[[^\]]*\]\([^)]*\)` → ' ' (markdown link pills).
    let mut out = String::with_capacity(stripped.len());
    let mut rest = stripped;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        out.push(' ');
        match rest[open + 1..].find("](").map(|i| open + 1 + i) {
            Some(close_open) => match rest[close_open + 2..].find(')') {
                Some(end) => {
                    let skip = close_open + 2 + end + 1;
                    rest = &rest[skip..];
                }
                None => {
                    out.push_str(&rest[open..]);
                    rest = "";
                }
            },
            None => {
                out.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    stripped = out.trim();
    // `^(?:@[\w.-]+[:,]?\s+)+` — leading @name address tokens.
    loop {
        let s = stripped.trim_start();
        if !s.starts_with('@') {
            break;
        }
        let localpart: String = s[1..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':'))
            .collect();
        if localpart.is_empty() {
            break;
        }
        let mut after = &s[1 + localpart.len()..];
        if let Some(r) = after.strip_prefix(':') {
            // skip the trailing `:` colon separator, then whitespace
            after = r;
        } else if let Some(r) = after.strip_prefix(',') {
            after = r;
        }
        if !after.trim_start().is_empty() && !after.starts_with(char::is_whitespace) {
            break;
        }
        stripped = after.trim_start();
    }
    stripped.to_owned()
}

/// The persisted session override (mirrors `router/src/store.ts:81-82`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionOverrides {
    pub model: Option<String>,
    pub mode: Option<ThreadMode>,
}

/// The session's model override, read directly from the row (no `DomainRepository`
/// needed, so the dispatch projection can use it). `None` = no override.
pub(super) fn model_override(
    db: &rusqlite::Connection,
    session: &str,
) -> Result<Option<String>, Error> {
    db.query_row(
        "SELECT model_override FROM runner_sessions WHERE id=?1",
        [session],
        |r| r.get::<_, Option<String>>(0),
    )
    .optional()
    .map(|found| found.flatten())
    .map_err(Error::from)
}

impl DomainRepository {
    /// Read the session's current overrides. `None` rows become `None` fields,
    /// matching the TS `?? null` projection (`store.ts:388-389`).
    pub fn session_overrides(&self, session: &str) -> Result<SessionOverrides, Error> {
        identifier(session, 128)?;
        let (model, mode): (Option<String>, Option<String>) = self
            .db
            .query_row(
                "SELECT model_override,mode_override FROM runner_sessions WHERE id=?1",
                [session],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        let mode = match mode.as_deref() {
            Some("plan") => Some(ThreadMode::Plan),
            Some("auto") => Some(ThreadMode::Auto),
            _ => None,
        };
        Ok(SessionOverrides { model, mode })
    }

    /// Apply an override, mirroring `setSessionOverrides` (`store.ts:656-695`):
    /// an undefined field keeps the existing override; a `default|reset|clear`
    /// verb arrives as `None` and clears it. The parser already refused a
    /// malformed model/mode, so this only persists a validated value.
    pub fn set_session_overrides(
        &mut self,
        session: &str,
        directive: &ThreadDirective,
    ) -> Result<SessionOverrides, Error> {
        identifier(session, 128)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: (Option<String>, Option<String>) = tx
            .query_row(
                "SELECT model_override,mode_override FROM runner_sessions WHERE id=?1",
                [session],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        let (model, mode) = match directive {
            ThreadDirective::Model(Some(m)) => (Some(m.clone()), existing.1),
            ThreadDirective::Model(None) => (None, existing.1),
            ThreadDirective::Mode(Some(ThreadMode::Plan)) => (existing.0, Some("plan".into())),
            ThreadDirective::Mode(Some(ThreadMode::Auto)) => (existing.0, Some("auto".into())),
            ThreadDirective::Mode(None) => (existing.0, None),
        };
        tx.execute(
            "UPDATE runner_sessions SET model_override=?2,mode_override=?3 WHERE id=?1",
            params![session, model, mode],
        )?;
        tx.commit()?;
        self.session_overrides(session)
    }

    /// The admitted `/thread` lines in this session with no answer queued yet,
    /// mirroring `pending_command_lines` (`command_notices.rs`) but gated on the
    /// directive shape instead of `!`. The host parses each, applies the
    /// override when authorized, and answers in-thread through the command-notice
    /// path. A line already answered is not offered again.
    pub fn pending_thread_directives(
        &self,
        session: &str,
        limit: i64,
    ) -> Result<Vec<hagency_core::commands::CommandLine>, Error> {
        identifier(session, 128)?;
        if !(1..=1024).contains(&limit) {
            return Err(hagency_core::InvalidInput("invalid directive line limit").into());
        }
        // Board #113, same defect as `pending_command_lines`: the limit used to
        // bound every `session_inputs` row and filter for `/thread` in Rust, so
        // a directive past the session's first `limit` inputs was dropped with
        // no trace. The prefilter tests the MESSAGE BODY only (a filter on the
        // whole config matches every row, because `room_id` starts with `!`),
        // and is a SUPERSET of `is_directive` — SQL never decides what a
        // directive is.
        let encoded: Vec<String> = self
            .db
            .prepare("SELECT CASE WHEN s.matrix_generation>0 THEN i.config ELSE m.config END FROM session_inputs i JOIN admitted_messages m ON m.sequence=i.message_sequence JOIN runner_sessions s ON s.id=i.session_id WHERE i.session_id=?1 AND instr(json_extract(CASE WHEN s.matrix_generation>0 THEN i.config ELSE m.config END,'$.body'),'/thread')>0 ORDER BY i.message_sequence LIMIT ?2")?
            .query_map(params![session, limit], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let mut lines = Vec::new();
        for encoded in encoded {
            let message: hagency_core::messages::Message = serde_json::from_str(&encoded)?;
            if !matches!(message.kind.as_str(), "m.text") || !is_directive(&message.body) {
                // Board #113: a skip must say why (see `pending_command_lines`).
                tracing::debug!(
                    session,
                    event_id = %message.event_id,
                    "thread directive skipped: not a `/thread` line"
                );
                continue;
            }
            let id = format!(
                "cmdn_{}",
                hagency_core::canonical::digest(&serde_json::json!([session, message.event_id]))?
            );
            let answered: bool = self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM command_notices WHERE id=?1)",
                [&id],
                |r| r.get(0),
            )?;
            if answered {
                tracing::debug!(
                    session,
                    event_id = %message.event_id,
                    "thread directive skipped: this session already queued an answer"
                );
                continue;
            }
            lines.push(hagency_core::commands::CommandLine {
                session_id: session.to_owned(),
                server_name: message.server_name,
                room_id: message.room_id,
                event_id: message.event_id,
                thread_root: message.thread_root,
                body: message.body,
                sender_mxid: message.sender_mxid,
            });
        }
        Ok(lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(body: &str) -> Option<Result<ThreadDirective, String>> {
        parse(body)
    }

    #[test]
    fn native_thread_directive_model_parses() {
        assert_eq!(
            model("/thread model claude-sonnet-5"),
            Some(Ok(ThreadDirective::Model(Some("claude-sonnet-5".into()))))
        );
        // Mention pill + bare @name are stripped so the directive leads.
        assert_eq!(
            model(
                "[@coordinator](https://matrix.to/#/@coordinator) /thread model claude-haiku-4-5"
            ),
            Some(Ok(ThreadDirective::Model(Some("claude-haiku-4-5".into()))))
        );
        assert_eq!(
            model("@coordinator /thread model claude-haiku-4-5"),
            Some(Ok(ThreadDirective::Model(Some("claude-haiku-4-5".into()))))
        );
        // default|reset|clear clears the override.
        for verb in ["default", "reset", "clear", "DEFAULT", "Clear"] {
            assert_eq!(
                model(&format!("/thread model {verb}")),
                Some(Ok(ThreadDirective::Model(None))),
                "{verb}"
            );
        }
    }

    #[test]
    fn native_thread_directive_mode_parses() {
        assert_eq!(
            model("/thread mode auto"),
            Some(Ok(ThreadDirective::Mode(Some(ThreadMode::Auto))))
        );
        assert_eq!(
            model("/thread mode plan"),
            Some(Ok(ThreadDirective::Mode(Some(ThreadMode::Plan))))
        );
        assert_eq!(
            model("/thread mode default"),
            Some(Ok(ThreadDirective::Mode(None)))
        );
    }

    #[test]
    fn native_thread_directive_malformed_is_usage_or_refusal() {
        // `/thread` alone answers with usage, not a delivery failure.
        assert_eq!(model("/thread"), Some(Err(THREAD_DIRECTIVE_USAGE.into())));
        assert_eq!(
            model("/thread model"),
            Some(Err(THREAD_DIRECTIVE_USAGE.into()))
        );
        assert_eq!(
            model("/thread mode maybe"),
            Some(Err(THREAD_DIRECTIVE_USAGE.into()))
        );
        // A single-token model with shell metacharacters is refused with the
        // model refusal (TS `THREAD_DIRECTIVE_MODEL_PATTERN`).
        assert_eq!(
            model("/thread model sonnet;rm"),
            Some(Err(THREAD_DIRECTIVE_MODEL_REFUSAL.into()))
        );
        // A multi-token model does not match `/thread\s+(\S+)(?:\s+(\S+))?\s*$`
        // → usage (the `; rm -rf /` case collapses to the two-token shape).
        assert_eq!(
            model("/thread model a b c"),
            Some(Err(THREAD_DIRECTIVE_USAGE.into()))
        );
    }

    #[test]
    fn native_thread_directive_not_a_directive_is_none() {
        assert_eq!(model("please take notes"), None);
        assert_eq!(model("thread model x"), None);
        // `/thread` must end at a word boundary (TS `/^\/thread\b/`).
        assert_eq!(model("/threadfoo"), None);
        assert_eq!(model("/threaded model x"), None);
    }

    #[test]
    fn native_thread_directive_confirmation_matches_ts() {
        assert_eq!(
            confirmation(Some("claude-sonnet-5"), None),
            "Thread session updated: model=claude-sonnet-5, mode=default (read-only)."
        );
        assert_eq!(
            confirmation(None, None),
            "Thread session updated: model=default, mode=default (read-only)."
        );
        assert_eq!(
            confirmation(None, Some(ThreadMode::Auto)),
            "Thread session updated: model=default, mode=auto. Runners in this thread may now write to the agent workspace (writes stay serialized by workspace lease)."
        );
        assert_eq!(
            confirmation(Some("claude-haiku-4-5"), Some(ThreadMode::Plan)),
            "Thread session updated: model=claude-haiku-4-5, mode=plan."
        );
    }
}
