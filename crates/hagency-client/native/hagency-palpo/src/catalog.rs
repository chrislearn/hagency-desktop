//! Coherent domain observations enter the existing immutable publication lane.
use super::*;
use hagency_store::DomainStore;

impl Adapter {
    pub async fn publish_resources_once(
        &self,
        domain: &DomainStore,
        cancel: &CancellationToken,
    ) -> Result<Step, Error> {
        let _guard = self.publication.try_lock().map_err(|_| Error::Busy)?;
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        // Rotation is checked even for an already frozen historical catalog.
        // A changed catalog alone must not overwrite that original pending body.
        domain
            .check_publication_registration(self.registration.clone())
            .await?;
        let Reply::Publication(pending) = self
            .command(Command::PendingPublication(self.scope()))
            .await?
        else {
            return Err(Error::Custody);
        };
        let mut included = Vec::new();
        if pending.is_none() {
            let snapshot = domain.published_catalog(self.registration.clone()).await?;
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let mut body = snapshot.into_update();
            if let Some(source) = &self.receipts {
                included = source.pending().into_iter().take(10).collect();
                if !included.is_empty() {
                    body["probeReceipts"] = Value::Array(included.clone());
                }
                let statuses: Vec<Value> = source.statuses().into_iter().take(200).collect();
                if !statuses.is_empty() {
                    body["statuses"] = Value::Array(statuses);
                }
            }
            let Reply::Publication(Some(_)) = self
                .command(Command::FreezePublication {
                    scope: self.scope(),
                    body,
                })
                .await?
            else {
                return Err(Error::Custody);
            };
        }
        let step = self.publish_checked(Some(domain), cancel).await?;
        if step == Step::Published
            && !included.is_empty()
            && let Some(source) = &self.receipts
        {
            source.published(&included);
        }
        Ok(step)
    }
}
