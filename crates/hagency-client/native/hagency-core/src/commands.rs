//! Host-authored answers to a `!` command line.
//!
//! A `!` line is never agent input (`hagency_store::domain::verified_ingress`'s
//! `is_bot_command` gates `wake` off): it is a bot command, and the retained
//! bridge answered it in the room that carried it, in the same thread, as an
//! `m.text` with an optional `org.matrix.custom.html` rendering
//! (`lib/bot-commands.js` `reply`/`sendInto`). Native answers it the same way,
//! except that the sender is the AGENT that received the line — it is the one
//! with an authenticated transport in that room and the Rust bot may not be a
//! member at all.
//!
//! There is no task and no owner approval behind that answer: the command layer
//! renders it. So it rides its own custody row (`command_notices`) and reuses
//! the final-reply transport rather than borrowing task or approval custody it
//! would have to lie to obtain.
use serde::{Deserialize, Serialize};

/// What to answer, as the command layer rendered it. The optional `html` is the
/// `org.matrix.custom.html` body the retained product attached for the commands
/// that had one (`!help`, `!status`, `!agents`, `!sessions`); the refusals were
/// plain-only and leave it `None`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandNotice {
    pub id: String,
    pub session_id: String,
    pub transaction_id: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    pub source_event_id: String,
}

/// Host-only intent to answer one admitted `!` line. The id is derived from the
/// session and the source event, so re-admitting the same line converges on the
/// same answer instead of sending a second one.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandNoticeRequest {
    pub session_id: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    pub source_event_id: String,
}

impl CommandNoticeRequest {
    pub fn validate(&self) -> Result<(), crate::InvalidInput> {
        use crate::InvalidInput;
        if self.session_id.is_empty() || self.session_id.len() > 128 {
            return Err(InvalidInput("invalid command notice session"));
        }
        if self.body.is_empty() || self.body.len() > 65_536 {
            return Err(InvalidInput("invalid command notice body"));
        }
        if self.html.as_ref().is_some_and(|html| html.len() > 524_288) {
            return Err(InvalidInput("invalid command notice html"));
        }
        crate::replies::matrix_event(&self.source_event_id)
            .map_err(|_| InvalidInput("invalid command notice source event"))
    }
}

/// One admitted `!` line in a session that has no answer queued yet. The
/// host renders it (`hagency::bot_commands`) and submits the answer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandLine {
    pub session_id: String,
    pub server_name: String,
    pub room_id: String,
    pub event_id: String,
    pub thread_root: Option<String>,
    pub body: String,
    /// The human the ACL is decided against. Taken from the admitted event, so
    /// it is the sender Matrix authenticated rather than anything the body says.
    pub sender_mxid: String,
}

/// Host transport capability; its secret is never persisted.
#[derive(Clone)]
pub struct CommandNoticeClaim {
    pub notice: CommandNotice,
    pub token: String,
    pub deadline: u64,
}

/// Host scheduling result. Only a successful one-shot begin authorizes an
/// adapter attempt; these fields alone never authorize a Matrix transmission.
/// Not `Serialize`: it carries the claim secret, which is never persisted and
/// never crosses the host boundary.
#[derive(Clone)]
pub struct CommandNoticeClaimed {
    pub claim: CommandNoticeClaim,
    pub route: crate::replies::ReplyRoute,
    pub digest: String,
}

/// Returned once after Sending commits.
#[derive(Clone, Serialize)]
pub struct CommandNoticeSend {
    pub notice: CommandNotice,
    pub route: crate::replies::ReplyRoute,
    pub digest: String,
    pub fence: u64,
}

/// No claim secret, route or body in this projection.
#[derive(Clone, Serialize)]
pub struct CommandNoticeReceipt {
    pub id: String,
    pub session_id: String,
    pub state: String,
    pub fence: u64,
    pub cancel_requested: bool,
    pub replayed: bool,
}
