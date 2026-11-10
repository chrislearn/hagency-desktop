use super::*;
use crate::documents::{MAX_FILE_BYTES, Text};
use serde_json::{Value, json};

pub(super) fn metadata(content: &Value) -> Option<Value> {
    if !matches!(content["msgtype"].as_str(), Some("m.file" | "m.image" | "m.audio" | "m.video")) {
        return None;
    }
    let filename = content
        .get("filename")
        .unwrap_or(&content["body"])
        .as_str()?;
    if filename.is_empty() || filename.len() > 1024 || filename.chars().any(char::is_control) {
        return None;
    }
    let mut result = json!({"filename":filename, "status":"not_read"});
    if let Some(mime) = content["info"]["mimetype"]
        .as_str()
        .filter(|m| m.len() <= 128 && !m.chars().any(char::is_control))
    {
        result["mimeType"] = mime.into();
    }
    if let Some(size) = content["info"]["size"].as_u64() {
        result["size"] = size.into();
    }
    Some(result)
}

fn source(
    content: &Value,
) -> Result<(hagency_matrix::MediaId, Option<hagency_media::Descriptor>), &'static str> {
    if content["info"]["size"]
        .as_u64()
        .is_some_and(|size| size > MAX_FILE_BYTES as u64)
    {
        return Err("file_size_limit");
    }
    let (url, descriptor) = if let Some(file) = content.get("file") {
        let mut fields = file.as_object().cloned().ok_or("invalid_attachment")?;
        let url = fields
            .remove("url")
            .and_then(|s| s.as_str().map(str::to_owned))
            .ok_or("invalid_attachment")?;
        let bytes = serde_json::to_vec(&fields).map_err(|_| "invalid_attachment")?;
        let descriptor = hagency_media::Descriptor::from_private_event_json(&bytes)
            .map_err(|_| "invalid_attachment")?;
        (url, Some(descriptor))
    } else {
        (
            content["url"].as_str().ok_or("invalid_attachment")?.into(),
            None,
        )
    };
    let id = hagency_matrix::MediaId::new(&url).map_err(|_| "invalid_attachment")?;
    Ok((id, descriptor))
}

pub(in crate::console) struct RoomFile {
    pub bytes: Vec<u8>,
    pub filename: String,
    pub mime: Option<String>,
}

impl AgentDevice {
    pub(in crate::console) async fn receive_room_file(&self, room: &str, event: &str) -> Result<RoomFile, &'static str> {
        if self.is_revoked() { return Err("attachment_unavailable"); }
        let room_id = OwnedRoomId::try_from(room).map_err(|_| "invalid_attachment")?;
        let event_id = OwnedEventId::try_from(event).map_err(|_| "invalid_attachment")?;
        let room = self
            .client
            .get_room(&room_id)
            .ok_or("attachment_unavailable")?;
        let event = room
            .event(&event_id, None)
            .await
            .map_err(|_| "attachment_unavailable")?;
        if event.kind.parse_event_id().as_ref() != Some(&event_id) {
            return Err("attachment_unavailable");
        }
        let record = history::history_event(&event).ok_or("attachment_unavailable")?;
        if record.get("status").is_some()
            || record.get("attachment").is_none()
        {
            return Err("attachment_unavailable");
        }
        let raw: Value = serde_json::from_str(event.kind.raw().json().get())
            .map_err(|_| "invalid_attachment")?;
        let content = &raw["content"];
        let (id, descriptor) = source(content)?;
        let token = self.client.access_token().ok_or("attachment_unavailable")?;
        let bytes = download(self.client.homeserver(), &token, &id).await?;
        let codec = hagency_media::Codec::new(
            hagency_media::Limits::new(MAX_FILE_BYTES, 1).map_err(|_| "invalid_attachment")?,
        );
        let checked = match descriptor {
            Some(descriptor) => codec.decrypt(&descriptor, &bytes),
            None => codec.check(bytes),
        }
        .map_err(|_| "attachment_integrity_failed")?;
        Ok(RoomFile {
            bytes: checked.bytes().to_vec(),
            filename: record["attachment"]["filename"].as_str().ok_or("invalid_attachment")?.into(),
            mime: record["attachment"]["mimeType"].as_str().map(str::to_owned),
        })
    }

    /// The caller selects only events already returned from this admitted
    /// room's history. Recheck current visibility, sender trust and redaction;
    /// obtain bytes using this Agent's credentials, never the user's session.
    pub(in crate::console) async fn read_document(
        &self,
        room: &str,
        event: &str,
    ) -> Result<Text, &'static str> {
        let file = self.receive_room_file(room, event).await?;
        let bytes = file.bytes.as_slice();
        let filename = file.filename.to_ascii_lowercase();
        let mime = file.mime.as_deref().unwrap_or_default().split(';').next().unwrap_or_default();
        if bytes.starts_with(b"%PDF-") || mime == "application/pdf" || filename.ends_with(".pdf") {
            return crate::documents::pdf(bytes.to_vec()).await;
        }
        if matches!(
            mime,
            "text/plain" | "text/markdown" | "text/csv" | "text/html" | "application/json"
        ) || [".txt", ".md", ".csv", ".json"]
            .iter()
            .any(|extension| filename.ends_with(extension))
        {
            return std::str::from_utf8(bytes)
                .map(Text::plain)
                .map_err(|_| "unsupported_text_encoding");
        }
        Err("unsupported_document_format")
    }
}

async fn download(
    mut homeserver: reqwest::Url,
    token: &str,
    id: &hagency_matrix::MediaId,
) -> Result<Vec<u8>, &'static str> {
    let mxc = id.to_mxc();
    let (server, media) = mxc
        .strip_prefix("mxc://")
        .and_then(|m| m.split_once('/'))
        .ok_or("invalid_attachment")?;
    homeserver.set_query(None);
    homeserver.set_fragment(None);
    homeserver
        .path_segments_mut()
        .map_err(|_| "invalid_attachment")?
        .clear()
        .extend([
            "_matrix", "client", "v1", "media", "download", server, media,
        ]);
    // An attachment chooses a repository ID, never the HTTP destination.
    // Refuse redirects so a repository cannot forward the Agent's bearer.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "attachment_download_failed")?;
    let mut response = client
        .get(homeserver)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| "attachment_download_failed")?;
    if !response.status().is_success() {
        return Err("attachment_download_failed");
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_FILE_BYTES as u64)
    {
        return Err("file_size_limit");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "attachment_download_failed")?
    {
        if chunk.len() > MAX_FILE_BYTES.saturating_sub(bytes.len()) {
            return Err("file_size_limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn attachment_adapter_decrypts_verified_bytes_and_rejects_tampering() {
        use std::io::Read;
        let original = b"%PDF-fixture-private-body";
        let mut input = std::io::Cursor::new(original);
        let mut encryptor = matrix_sdk_crypto::AttachmentEncryptor::new(&mut input);
        let mut ciphertext = Vec::new();
        encryptor.read_to_end(&mut ciphertext).unwrap();
        let mut file = serde_json::to_value(encryptor.finish()).unwrap();
        file["url"] = "mxc://example.test/encrypted".into();
        let content = json!({"msgtype":"m.file","body":"设计.pdf","file":file});
        let (id, descriptor) = source(&content).unwrap();
        assert_eq!(id.to_mxc(), "mxc://example.test/encrypted");
        let descriptor = descriptor.unwrap();
        let codec = hagency_media::Codec::new(hagency_media::Limits::new(MAX_FILE_BYTES, 1).unwrap());
        assert_eq!(codec.decrypt(&descriptor, &ciphertext).unwrap().bytes(), original);
        ciphertext[0] ^= 1;
        assert!(matches!(codec.decrypt(&descriptor, &ciphertext), Err(hagency_media::Error::Integrity)));
    }
    #[test]
    fn attachment_metadata_contains_no_download_or_decryption_authority() {
        let content = json!({"msgtype":"m.file","body":"设计.pdf","info":{"mimetype":"application/pdf","size":100},"url":"mxc://example.test/media","file":{"url":"mxc://example.test/encrypted","key":"secret","iv":"secret"}});
        assert_eq!(
            metadata(&content).unwrap(),
            json!({"filename":"设计.pdf","mimeType":"application/pdf","size":100,"status":"not_read"})
        );
        // A malformed encrypted descriptor cannot fall back to a plaintext URL.
        assert!(source(&content).is_err());
        for url in [
            "https://example.test/file",
            "file:///private/file",
            "mxc://example.test/../secret",
        ] {
            assert!(source(&json!({"url":url})).is_err());
        }
        assert!(
            source(&json!({"url":"mxc://example.test/media","info":{"size":MAX_FILE_BYTES+1}}))
                .is_err()
        );
    }
    #[tokio::test]
    async fn attachment_download_rejects_redirects_and_oversized_bodies() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for headers in ["HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/secret\r\nContent-Length: 0\r\n\r\n".into(), format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", MAX_FILE_BYTES + 1)] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/", listener.local_addr().unwrap()).parse().unwrap();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let size = stream.read(&mut request).await.unwrap();
                let request = std::str::from_utf8(&request[..size]).unwrap();
                assert!(request.starts_with("GET /_matrix/client/v1/media/download/example.test/media "));
                stream.write_all(headers.as_bytes()).await.unwrap();
            });
            assert!(download(url, "fixture-token", &hagency_matrix::MediaId::new("mxc://example.test/media").unwrap()).await.is_err());
            server.await.unwrap();
        }
    }
}
