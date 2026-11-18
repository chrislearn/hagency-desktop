//! Discover a recipient through the same signature-checked SDK store used to send.
use matrix_sdk::{Client, ruma::UserId};
use std::time::Duration;

/// Wait for a usable recipient device without overriding trust or sharing history.
/// A standalone HTTP `/keys/query` cannot populate the sender's SDK device cache.
pub async fn wait_for_sendable_device(
    client: &Client,
    user: &UserId,
) -> Result<(), matrix_sdk::Error> {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let devices = client.encryption().get_user_devices(user).await?;
            if devices.devices().any(|device| {
                !device.is_deleted()
                    && !device.is_blacklisted()
                    && device.curve25519_key().is_some()
            }) {
                return Ok(());
            }
            // This updates the SDK store after verifying device signatures.
            // A recipient need not have a cross-signing identity.
            client.encryption().request_user_identity(user).await?;
            let devices = client.encryption().get_user_devices(user).await?;
            if devices.devices().any(|device| {
                !device.is_deleted()
                    && !device.is_blacklisted()
                    && device.curve25519_key().is_some()
            }) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .map_err(|_| matrix_sdk::Error::Timeout)?
}
