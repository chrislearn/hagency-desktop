//! Per-room trust classifier (task #80): the TS `bridge-matrix.js`
//! `getRoomTrust`/`markRoomTrusted` (bridge-matrix.js:2841, 2854) parity, framed
//! under ADR-054 frozen targets — native's rooms are an agent decision, not the
//! operator's, so "managed" means "a room this service is configured to serve"
//! (a frozen target, `HostConfig.rooms` / `reception_room`) or "a room the
//! service marked trusted and persisted" (the store `room_trust` table, the TS
//! `trustedManagedRooms` record). The operator-facing allowlist
//! (`MATRIX_TRUSTED_ROOM_IDS`) and trusted-inviter list
//! (`MATRIX_TRUSTED_INVITER_MXIDS`) are the TS equivalents.
use crate::Error;

/// One trust verdict: TS returns `{trusted, reason}`; the reason is a stable
/// word the audit log and the enforce gate both branch on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomTrustReason {
    Allowlist,
    Managed,
    TrustedInviter,
    UnknownRoom,
}

impl RoomTrustReason {
    pub fn word(self) -> &'static str {
        match self {
            Self::Allowlist => "allowlist",
            Self::Managed => "managed",
            Self::TrustedInviter => "trusted_inviter",
            Self::UnknownRoom => "unknown_room",
        }
    }
    pub fn trusted(self) -> bool {
        !matches!(self, Self::UnknownRoom)
    }
}

/// The trust mode: TS `MATRIX_TRUST_MODE` defaults to `audit` (log untrusted,
/// process anyway) and only `enforce` refuses. Anything else is treated as audit
/// (TS prints a warning; native returns audit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustMode {
    Audit,
    Enforce,
}

impl TrustMode {
    pub fn from_env() -> Self {
        Self::from_word(
            std::env::var("MATRIX_TRUST_MODE")
                .unwrap_or_default()
                .as_str(),
        )
    }
    /// TS `MATRIX_TRUST_MODE` resolution: only `enforce` enforces; the default
    /// and every other spelling are audit.
    pub fn from_word(word: &str) -> Self {
        match word.trim().to_lowercase().as_str() {
            "enforce" => Self::Enforce,
            // TS default is 'audit'; every other spelling is treated as audit.
            _ => Self::Audit,
        }
    }
}

/// The classifier's inputs. Constructed once per host from its frozen config +
/// the env, then consulted per room.
pub struct RoomTrust {
    allowlist: std::collections::BTreeSet<String>,
    trusted_inviters: std::collections::BTreeSet<String>,
    /// Frozen targets the service manages (HostConfig.rooms + reception_room).
    managed: std::collections::BTreeSet<String>,
    /// Persisted trusted rooms (TS `trustedManagedRooms`, store `room_trust`).
    marked: std::collections::BTreeSet<String>,
    pub mode: TrustMode,
}

fn split_env(name: &str) -> std::collections::BTreeSet<String> {
    std::env::var(name)
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

impl RoomTrust {
    pub fn new(
        managed: impl IntoIterator<Item = String>,
        marked: impl IntoIterator<Item = String>,
    ) -> Self {
        Self::with_sets(
            split_env("MATRIX_TRUSTED_ROOM_IDS"),
            split_env("MATRIX_TRUSTED_INVITER_MXIDS"),
            managed,
            marked,
            TrustMode::from_env(),
        )
    }

    /// Test/direct-construction path: every set is supplied, no env is read.
    pub fn with_sets(
        allowlist: std::collections::BTreeSet<String>,
        trusted_inviters: std::collections::BTreeSet<String>,
        managed: impl IntoIterator<Item = String>,
        marked: impl IntoIterator<Item = String>,
        mode: TrustMode,
    ) -> Self {
        Self {
            allowlist,
            trusted_inviters,
            managed: managed.into_iter().collect(),
            marked: marked.into_iter().collect(),
            mode,
        }
    }

    /// TS `getRoomTrust(roomId, {inviterMxid})` parity: the same precedence
    /// (allowlist → managed → trusted inviter → unknown).
    pub fn classify(&self, room_id: &str, inviter_mxid: Option<&str>) -> RoomTrustReason {
        if self.allowlist.contains(room_id) {
            return RoomTrustReason::Allowlist;
        }
        if self.managed.contains(room_id) || self.marked.contains(room_id) {
            return RoomTrustReason::Managed;
        }
        if let Some(inviter) = inviter_mxid
            && self.trusted_inviters.contains(inviter)
        {
            return RoomTrustReason::TrustedInviter;
        }
        RoomTrustReason::UnknownRoom
    }

    /// The enforce gate: under `enforce`, an untrusted room's messages are not
    /// processed; under `audit` they are (only logged).
    pub fn admit(
        &self,
        room_id: &str,
        inviter_mxid: Option<&str>,
    ) -> Result<RoomTrustReason, Error> {
        let reason = self.classify(room_id, inviter_mxid);
        if self.mode == TrustMode::Enforce && !reason.trusted() {
            return Err(Error::Config);
        }
        Ok(reason)
    }
}
