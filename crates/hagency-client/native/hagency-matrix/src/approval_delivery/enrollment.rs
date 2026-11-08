use super::jobs::Value;
use crate::{
    ApprovalCollector, CancellationToken, Error, collector::Inner, enrollment::Scope, sdk::Owner,
};
use std::{
    collections::BTreeSet,
    sync::{Arc, atomic::AtomicBool},
};
use tokio::time::Instant;
impl ApprovalCollector {
    /// Explicit fresh ordinary-user account enrollment for this approval purpose.
    /// No Agent transport, permission decision or card transmission is admitted.
    ///
    /// ADR-183: a refusal — recipients, identity, transport, anything — fences
    /// nothing and does not retire this collector. The SDK's own ledger says
    /// what the next call may redo (a refused verify: fresh Query then Verify)
    /// and what stays uncertain (a write that may have crossed the wire), so
    /// the caller retries from the top once the fact changes.
    pub async fn enroll_fresh_account(&self, cancel: &CancellationToken) -> Result<(), Error> {
        if self.inner.config.enrollment.is_none() {
            return Err(Error::Config);
        }
        let permit = self.delivery_permit(false)?;
        let deadline = Instant::now() + self.inner.config.limits.sdk;
        let inner = self.inner.clone();
        let engagements = self.engagements.snapshot()?;
        let cancel = cancel.child_token();
        // The enrollment's custody lives in the SDK ledger, not in this job
        // registry: a returned refusal never blocks the next attempt.
        let retryable = Arc::new(AtomicBool::new(false));
        let job=self.jobs.start_classified(false,false,permit,retryable,async move{
            let work=async{
                inner.approval_enrollment_current(&engagements,&cancel).await?;
                let mut guard=inner.owner.lock().await;
                if guard.is_none(){*guard=Some(Owner::open(&inner.config).await?);}
                drop(guard);
                inner.enroll(Scope::Approval(&engagements),&cancel,deadline).await
            };
            tokio::pin!(work);
            let result=tokio::select!{r=&mut work=>r,_=tokio::time::sleep_until(deadline)=>{cancel.cancel();work.await}};
            result.map(|()| Value::Unit)
        })?;
        match job.wait().await? {
            Value::Unit => Ok(()),
            _ => Err(Error::Storage),
        }
    }
}
impl Inner {
    pub(crate) async fn approval_enrollment_current(
        &self,
        engagements: &[String],
        cancel: &CancellationToken,
    ) -> Result<Vec<String>, Error> {
        let rooms = self.approval_rooms(engagements).await?;
        self.refresh_approval_rooms(&rooms, cancel).await?;
        // Re-read original domain authority after all observed/published snapshots.
        let current = self.approval_rooms(engagements).await?;
        if rooms.len() != current.len()
            || rooms.iter().zip(&current).any(|(a, b)| {
                a.authority != b.authority || a.device != b.device || a.generation != b.generation
            })
        {
            return Err(Error::Generation);
        }
        let mut users = BTreeSet::new();
        for room in &current {
            let capture = self
                .domain
                .approval_room_capture(room.authority.clone())
                .await?
                .ok_or(Error::Generation)?;
            if !capture.available
                || capture.device_id != room.device
                || capture.generation != room.generation
            {
                return Err(Error::Generation);
            }
            users.extend([
                room.authority.bot_mxid.clone(),
                room.authority.owner_mxid.clone(),
            ]);
        }
        let users: Vec<_> = users.into_iter().collect();
        self.config
            .enrollment
            .as_ref()
            .ok_or(Error::Config)?
            .users(&self.config.identity.transport.sender_mxid, &users)?;
        Ok(users)
    }
}
