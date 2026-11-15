//! The uncertainty time bound for a claimed original runtime (ADR-193 for v1).
//! A launch can fail after its claim and before any process reports, for
//! example when preparation used up the issuance window. That leaves no
//! evidence: the server keeps the runtime issued, asks for cleanup, and the
//! Agent stays stopping, because a claimed command can never be proven not
//! started. After the same grace as an attempt's, and only while no custody in
//! this process still holds the claim, the claim is recorded as stopped under
//! its own process identity. Its process never qualified, so it never had a
//! model-call right; successors chain on this stop as on any other.
use super::*;

impl OwnerResourceRegistry {
    /// None while the command was never claimed (only the non-issuance proof
    /// applies then), its claim is held by live custody here, any process or
    /// call evidence exists, or the grace has not passed.
    pub async fn record_shared_runtime_timeout(
        &self,
        a: ManagedCleanupAuthority,
        c: SharedRuntimeCommand,
    ) -> Result<Option<SharedProcessReceipt>, Error> {
        a.check(&self.0)?;
        c.validate(&a.enrollment.profile.account)?;
        if a.child != c.agent_reservation_id {
            return Err(Error::LocalAuthority);
        }
        let profile = a.enrollment.profile.clone();
        let expires = a.deadline();
        let registry = self.0.clone();
        let value = self.0.store.call(64 * 1024, move |db| {
            a.check(&registry)?;
            let tx = db.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            authority::cleanup(&tx, &a)?;
            claim::original_command(&tx, &a.enrollment, &c)?;
            let digest = canonical::digest(&serde_json::to_value(&c)?)?;
            let claimed: Option<(String, String, String, String)> = tx.query_row(
                "SELECT profile,command_digest,incarnation,provider_digest FROM owner_shared_runtime_lifetimes WHERE child_id=?1",
                [&c.agent_reservation_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            ).optional()?;
            let Some((saved_profile, saved, incarnation, provider)) = claimed else {
                a.check(&registry)?;
                return Ok(None);
            };
            if saved_profile != a.enrollment.profile.digest || saved != digest {
                return Err(Error::Conflict);
            }
            // Any process evidence, successor or call means a process spoke
            // for this claim: its own evidence decides, never this bound. A
            // non-issuance proof counts only for this command: an earlier
            // original of the same Agent that was declined before any claim
            // (as when a start missed its window) says nothing about this one.
            let evidence: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM owner_shared_process_observations WHERE child_id=?1) \
                 OR EXISTS(SELECT 1 FROM owner_shared_unstarted_runtimes WHERE child_id=?1 AND command_id=?2) \
                 OR EXISTS(SELECT 1 FROM owner_shared_successor_attempts WHERE child_id=?1) \
                 OR EXISTS(SELECT 1 FROM owner_shared_successor_fences WHERE child_id=?1) \
                 OR EXISTS(SELECT 1 FROM owner_managed_runtime_lifetimes WHERE child_id=?1) \
                 OR EXISTS(SELECT 1 FROM owner_managed_calls WHERE child_id=?1)",
                params![c.agent_reservation_id, c.command_id],
                |r| r.get(0),
            )?;
            if evidence
                || claim::live(&registry, &incarnation)
                || !attempt::timeout_due(c.expires_at_ms)?
            {
                a.check(&registry)?;
                return Ok(None);
            }
            let process = canonical::digest(&json!({"commandDigest":digest,"incarnation":incarnation,"providerDigest":provider}))?;
            let id = format!("spro_{}", canonical::digest(&json!({"processDigest":process,"state":"stopped","policyDigest":null,"reason":UNCERTAIN_TIMEOUT}))?);
            let revision = revision(&tx)?;
            let receipt = SharedProcessReceipt {
                version: 1,
                observation_id: id.clone(),
                command_id: c.command_id.clone(),
                command_digest: digest,
                assignment_id: c.assignment_id.clone(),
                agent_reservation_id: c.agent_reservation_id.clone(),
                ledger_revision: revision,
                process_digest: Some(process),
                state: "stopped".into(),
                policy_digest: None,
            };
            receipt.validate_for(&c)?;
            bounded_row(&tx, "owner_shared_process_observations", "id", &id, 100_000)?;
            tx.execute(
                "INSERT INTO owner_shared_process_observations VALUES(?1,?2,?3,?4,?5)",
                params![id, c.agent_reservation_id, canonical::encode(&serde_json::to_value(&receipt)?)?, canonical::digest(&serde_json::to_value(&receipt)?)?, revision],
            )?;
            observations::close_stopped_calls(&tx, &c.agent_reservation_id)?;
            a.check(&registry)?;
            tx.commit()?;
            a.check(&registry).map_err(|_| Error::OutcomeUnknown)?;
            Ok(Some(receipt))
        }).await?;
        after(&profile, &self.0, expires)?;
        Ok(value)
    }
}
