//! Local operator command; its output contains only the short-lived ticket
//! that opens the console. One link is the whole console (TS parity: the
//! retained `createApiAuthMiddleware` admitted one credential to every `/api`
//! route), so there is no scope to select here.
use super::Error;
use http_body_util::{BodyExt, Full};
use hyper::{Request, body::Bytes, client::conn::http1};
use hyper_util::rt::TokioIo;
use serde::Deserialize;
use std::{net::SocketAddr, path::Path, time::Duration};
use tokio::net::TcpStream;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Issued {
    ticket: String,
    expires_in: u64,
}
/// One link opens the whole console. The retained middleware authenticated a
/// single credential for every `/api` route, so there is no scope to request
/// and no landing-page selection: the operator lands on the usage page, the
/// one the bare command always printed.
pub async fn access(state: &Path, address: SocketAddr) -> Result<String, Error> {
    if !address.ip().is_loopback()
        || address.port() == 0
        || matches!(address, SocketAddr::V6(v) if v.scope_id()!=0 || v.flowinfo()!=0)
    {
        return Err(Error::Invalid);
    }
    let token = hagency_store::private::read_secret(&state.join("operator.token"))
        .map_err(|_| Error::Unavailable)?;
    let token = std::str::from_utf8(&token).map_err(|_| Error::Unavailable)?;
    if !(32..=256).contains(&token.len()) || !token.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(Error::Unavailable);
    }
    tokio::time::timeout(Duration::from_secs(5), exchange(address, token))
        .await
        .map_err(|_| Error::Unavailable)?
}
/// Whether the service answers on `address` (a TCP connect), without
/// issuing a sign-in link.
pub async fn reachable(address: SocketAddr) -> bool {
    matches!(
        tokio::time::timeout(Duration::from_secs(1), TcpStream::connect(address)).await,
        Ok(Ok(_))
    )
}
async fn exchange(address: SocketAddr, token: &str) -> Result<String, Error> {
    let stream = TcpStream::connect(address)
        .await
        .map_err(|_| Error::Unavailable)?;
    let (mut sender, connection) = http1::Builder::new()
        .max_headers(32)
        .max_buf_size(16 * 1024)
        .handshake::<_, Full<Bytes>>(TokioIo::new(stream))
        .await
        .map_err(|_| Error::Unavailable)?;
    let mut authorization = hyper::header::HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| Error::Invalid)?;
    authorization.set_sensitive(true);
    let request = Request::builder()
        .method("POST")
        // One login route, no scope selection (TS parity: the retained
        // middleware authenticated one credential for every `/api` route).
        .uri("/api/native/v1/console/access")
        .header("host", address.to_string())
        .header("authorization", authorization)
        .header("connection", "close")
        .body(Full::new(Bytes::new()))
        .map_err(|_| Error::Invalid)?;
    let read = async {
        let mut response = sender
            .send_request(request)
            .await
            .map_err(|_| Error::Unavailable)?;
        if response.status().as_u16() != 200
            || response.headers().contains_key("content-encoding")
            || response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .is_none_or(|v| !v.starts_with("application/json"))
        {
            return Err(Error::Unavailable);
        }
        let mut bytes = Vec::new();
        while let Some(frame) = response.body_mut().frame().await {
            let data = frame
                .map_err(|_| Error::Unavailable)?
                .into_data()
                .map_err(|_| Error::Unavailable)?;
            if bytes.len().saturating_add(data.len()) > 256 {
                return Err(Error::Unavailable);
            }
            bytes.extend_from_slice(&data);
        }
        let issued: Issued = serde_json::from_slice(&bytes).map_err(|_| Error::Unavailable)?;
        if issued.expires_in != 120
            || issued.ticket.len() != 64
            || !issued
                .ticket
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(Error::Unavailable);
        }
        // The landing page the bare command always printed. With one link
        // there is no scope to steer by, and this keeps the surviving
        // command's behaviour rather than inventing a new landing.
        Ok(format!(
            "http://{address}/console/usage/#access={}",
            issued.ticket
        ))
    };
    tokio::pin!(read, connection);
    tokio::select! {
        result = &mut read => result,
        result = &mut connection => { result.map_err(|_| Error::Unavailable)?; read.await }
    }
}
