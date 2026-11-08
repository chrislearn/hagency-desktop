//! Owner state custody and headless management host for Desktop and its daemon.
use crate::console::Console;
use salvo::prelude::*;
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
const MARKER: &str = "hagency-client-owned-v1.json";
const MARKER_BYTES: &[u8] = b"{\"format\":\"hagency-owned-agent-client\",\"version\":1}\n";
const LOCK: &str = ".owner-client.lock";
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("use a fresh private owner-client state directory; legacy or foreign data is refused")]
    State,
    #[error("another owner-client process is using this state directory")]
    AlreadyRunning,
    #[error("use a nonzero loopback listen address")]
    Address,
    #[error("native owner management is unavailable")]
    Management,
    #[error("owner-client HTTP listener failed")]
    Server,
}
struct State {
    path: PathBuf,
    _lock: std::fs::File,
    directory: std::fs::File,
}
fn allowed_entry(name: &str) -> bool {
    // Accept a retired browser socket left in an existing root; never open or recreate it.
    matches!(name,MARKER|LOCK|"server-login.json"|"server-login-sdk-resume.json"|"server-login-revocations.json"|"server-login-profiles.json"|"owned-agent-owners"|"shared-matrix-devices"|"owner-console.sock")
        || (cfg!(feature = "resource-adoption") && name == "adopted-management-initializer-v1.json")
        // Interrupted private JSON replacement files are never opened/imported.
        || (name.len()==21 && name.starts_with('.') && name.ends_with(".tmp") && name.as_bytes()[1..17].iter().all(|b|b.is_ascii_hexdigit()))
}
fn prepare(path: &Path) -> Result<State, Error> {
    if !path.is_absolute() {
        return Err(Error::State);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| Error::State)?;
    }
    hagency_store::private::directory(path).map_err(|_| Error::State)?;
    let path = path.canonicalize().map_err(|_| Error::State)?;
    let existing: Vec<_> = std::fs::read_dir(&path)
        .map_err(|_| Error::State)?
        .collect::<Result<_, _>>()
        .map_err(|_| Error::State)?;
    let has_marker = path.join(MARKER).exists();
    // No old database/config bytes are read. Absence of our fresh format marker
    // permits only an empty directory, including after interrupted lock creation.
    if (!has_marker && existing.iter().any(|e| e.file_name() != LOCK))
        || existing
            .iter()
            .any(|e| !e.file_name().to_str().is_some_and(allowed_entry))
    {
        return Err(Error::State);
    }
    let lock = hagency_store::private::open(&path.join(LOCK), true)
        .or_else(|_| hagency_store::private::open(&path.join(LOCK), false))
        .map_err(|_| Error::State)?;
    lock.try_lock().map_err(|_| Error::AlreadyRunning)?;
    if has_marker {
        let bytes =
            hagency_store::private::read_secret(&path.join(MARKER)).map_err(|_| Error::State)?;
        if bytes != MARKER_BYTES {
            return Err(Error::State);
        }
    } else {
        hagency_store::private::replace(&path.join(MARKER), MARKER_BYTES)
            .map_err(|_| Error::State)?;
    }
    let directory = cap_std::fs::Dir::open_ambient_dir(&path, cap_std::ambient_authority())
        .map_err(|_| Error::State)?
        .into_std_file();
    Ok(State {
        path,
        _lock: lock,
        directory,
    })
}
/// Initializes only the new format, and never imports old SQLite or credentials.
pub fn initialize(path: &Path) -> Result<PathBuf, Error> {
    Ok(prepare(path)?.path)
}
/// Whether an existing directory is one `prepare` would accept: empty (or an
/// interrupted lock only) or this client's format. Reads only the marker;
/// takes no lock and writes nothing.
#[cfg(unix)]
pub(crate) fn is_owner_root_format(path: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(path).and_then(|d| d.collect::<Result<Vec<_>, _>>()) else {
        return false;
    };
    let names: Vec<_> = entries.iter().map(|e| e.file_name()).collect();
    if names.iter().any(|n| !n.to_str().is_some_and(allowed_entry)) {
        return false;
    }
    if !names.iter().any(|n| n == MARKER) {
        return names.iter().all(|n| n == LOCK);
    }
    hagency_store::private::read_secret(&path.join(MARKER)).is_ok_and(|b| b == MARKER_BYTES)
}
/// A reduction-only native operation on an already initialized, exclusively
/// owned root. Does not construct a console, database, listener or new root.
pub(crate) fn with_existing_state<T>(
    path: &Path,
    action: impl FnOnce(&Path) -> T,
) -> Result<Option<T>, Error> {
    if !path.is_absolute() {
        return Err(Error::State);
    }
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Error::State),
        Ok(m) if !m.is_dir() || m.file_type().is_symlink() => return Err(Error::State),
        Ok(_) => (),
    }
    if hagency_store::private::read_secret(&path.join(MARKER)).map_err(|_| Error::State)?
        != MARKER_BYTES
    {
        return Err(Error::State);
    }
    let state = prepare(path)?;
    Ok(Some(action(&state.path)))
}
#[derive(Clone)]
pub struct OwnerHost {
    pub(crate) console: Console,
    pub(crate) authority: String,
    state: Arc<State>,
}
impl OwnerHost {
    /// Open a headless owner host. No browser assets or browser access IPC exist.
    pub fn open(state: &Path, address: SocketAddr) -> Result<Self, Error> {
        Self::open_native(state, address)
    }
    /// Local process authority used by the in-process adapter and test clients.
    pub fn access_ticket(&self) -> Result<String, Error> {
        self.console.owner_access_ticket().map_err(|_| Error::Management)
    }
    /// In-process native management. No web assets are loaded or presented.
    pub(crate) fn open_native(state: &Path, address: SocketAddr) -> Result<Self, Error> {
        if !address.ip().is_loopback() || address.port() == 0 {
            return Err(Error::Address);
        }
        Self::native_with_authority(state, address.to_string())
    }
    /// The embedding application's Matrix SDK owns authentication. This host
    /// has no socket or browser callback. Its reserved authority is used only
    /// by the in-process request adapter and can never be passed to `serve`.
    pub(crate) fn open_embedded(state: &Path) -> Result<Self, Error> {
        Self::native_with_authority(state, "hagency-native.invalid".into())
    }
    fn native_with_authority(state: &Path, authority: String) -> Result<Self, Error> {
        let state = prepare(state)?;
        let console = Console::native_with_state(&state.path).map_err(|_| Error::Management)?;
        Ok(Self {
            console,
            authority,
            state: Arc::new(state),
        })
    }
    pub fn state_directory(&self) -> &Path {
        &self.state.path
    }
    /// Retaining this host is root-lock custody only while the physical root
    /// and lock path still identify the original private handles.
    pub(crate) fn check_state_lock(&self) -> Result<(), Error> {
        use cap_fs_ext::DirExt;
        let parent = cap_std::fs::Dir::open_ambient_dir(
            self.state.path.parent().ok_or(Error::State)?,
            cap_std::ambient_authority(),
        )
        .map_err(|_| Error::State)?;
        let current = parent
            .open_dir_nofollow(self.state.path.file_name().ok_or(Error::State)?)
            .map_err(|_| Error::State)?
            .into_std_file();
        let lock = hagency_store::private::open(&self.state.path.join(LOCK), false)
            .map_err(|_| Error::State)?;
        if !hagency_platform::same_directory(&self.state.directory, &current)
            .map_err(|_| Error::State)?
            || !hagency_platform::same_file(&self.state._lock, &lock).map_err(|_| Error::State)?
        {
            return Err(Error::State);
        }
        hagency_store::private::check_handle(&current).map_err(|_| Error::State)?;
        hagency_store::private::check_handle(&lock).map_err(|_| Error::State)
    }
    pub fn router(self) -> Router {
        Router::new()
            .hoop(self)
            .push(Router::with_path("health").get(health))
            .push(Router::with_path("ready").get(health))
            .push(crate::console::owner_router())
    }
    pub async fn serve(
        self,
        address: SocketAddr,
        shutdown: &hagency_matrix::CancellationToken,
    ) -> Result<(), Error> {
        if self.authority != address.to_string() {
            return Err(Error::Address);
        }
        let acceptor = TcpListener::new(address)
            .try_bind()
            .await
            .map_err(|_| Error::Server)?;
        let server = Server::new(acceptor).max_connections(64);
        let handle = server.handle();
        let mut serving = Box::pin(server.try_serve(self.clone().router()));
        let result = tokio::select! { result=&mut serving=>result.map_err(|_|Error::Server),_=shutdown.cancelled()=>Ok(())};
        self.console.retire();
        self.console.stop_owned_runtimes().await;
        self.console.stop_owner_provider().await;
        handle.stop_graceful(Some(Duration::from_secs(5)));
        if result.is_ok() {
            serving.await.map_err(|_| Error::Server)?;
        }
        result
    }
}

#[handler]
impl OwnerHost {
    async fn handle(&self, depot: &mut Depot) {
        depot.insert_typed(self.clone());
    }
}
#[handler]
async fn health(res: &mut Response) {
    res.render(Json(
        serde_json::json!({"ok":true,"mode":"owned-agent-client","fleet":false}),
    ));
}
#[cfg(test)]
mod tests {
    use super::*;
    fn temporary() -> tempfile::TempDir {
        let temp = hagency_platform::legacy_writer::test_support::initialized_tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        temp
    }
    #[test]
    fn old_state_and_foreign_marker_are_refused_without_reading_or_rewriting_old_data() {
        for name in [
            "domain.sqlite3",
            "custody.sqlite3",
            "operator.token",
            "fleet-runtime.json",
            "task_graphs.json",
        ] {
            let temp = temporary();
            let old = temp.path().join(name);
            std::fs::write(&old, b"legacy sentinel").unwrap();
            assert!(matches!(initialize(temp.path()), Err(Error::State)));
            assert_eq!(std::fs::read(old).unwrap(), b"legacy sentinel");
            assert!(!temp.path().join(MARKER).exists());
            assert!(!temp.path().join(LOCK).exists());
        }
        let temp = temporary();
        std::fs::write(temp.path().join(MARKER), b"foreign").unwrap();
        assert!(initialize(temp.path()).is_err());
    }
    #[test]
    fn fresh_format_is_reopenable_and_has_one_process_owner() {
        let temp = temporary();
        let state = prepare(temp.path()).unwrap();
        assert_eq!(
            std::fs::read(state.path.join(MARKER)).unwrap(),
            MARKER_BYTES
        );
        assert!(matches!(prepare(temp.path()), Err(Error::AlreadyRunning)));
        drop(state);
        assert!(initialize(temp.path()).is_ok());
        assert!(!temp.path().join("domain.sqlite3").exists());
        assert!(!temp.path().join("operator.token").exists());
    }
    #[test]
    fn shared_matrix_device_storage_survives_owner_restart() {
        let temp = temporary();
        let state = prepare(temp.path()).unwrap();
        let devices = state.path.join("shared-matrix-devices");
        hagency_store::private::directory(&devices).unwrap();
        let store = devices.join("device-store");
        hagency_store::private::replace(&store, b"retained crypto store").unwrap();
        drop(state);
        let reopened = prepare(temp.path()).unwrap();
        assert_eq!(hagency_store::private::read_secret(&store).unwrap(), b"retained crypto store");
        drop(reopened);
        std::fs::write(temp.path().join("foreign-state"), b"foreign").unwrap();
        assert!(matches!(prepare(temp.path()), Err(Error::State)));
    }

    #[tokio::test]
    async fn headless_owner_serves_api_without_ui_assets_or_browser_access_socket() {
        let temp = temporary();
        let state = temp.path().join("state");
        let address: SocketAddr = "127.0.0.1:13300".parse().unwrap();
        let host = OwnerHost::open(&state, address).unwrap();
        let service = Service::new(host.clone().router());
        for path in [
            "/console/",
            "/console/login/",
            "/console/agents-owned/",
            "/console/_next/static/app.js",
            "/api/native/v1/resources",
            "/api/native/v1/custody",
            "/api/native/v1/fleet",
            "/api/native/v1/console/access",
            "/runner",
            "/console/usage/",
            "/console/resources/",
            "/console/engagements/",
            "/console/api/engagements",
            "/console/api/accounts",
            "/console/api/approvals",
            "/console/api/resource-configuration",
        ] {
            let response = salvo::test::TestClient::get(format!("http://{address}{path}"))
                .add_header("host", address.to_string(), true)
                .add_header("sec-fetch-site", "same-origin", true)
                .send(&service).await;
            assert_eq!(response.status_code, Some(StatusCode::NOT_FOUND), "{path}");
        }
        let response = salvo::test::TestClient::get(format!("http://{address}/console/api/owned-agents"))
            .add_header("host", address.to_string(), true)
            .add_header("sec-fetch-site", "same-origin", true)
            .send(&service).await;
        assert_eq!(response.status_code, Some(StatusCode::UNAUTHORIZED));
        assert!(!state.join("owner-console.sock").exists());
        assert!(!state.join("domain.sqlite3").exists());
        assert!(!state.join("operator.token").exists());
        assert!(!state.join("task_graphs.json").exists());
        host.console.retire();
        host.console.stop_owned_runtimes().await;
        host.console.stop_owner_provider().await;
    }

    #[tokio::test]
    async fn local_access_ticket_never_grants_server_owner_authorization() {
        // Preserve the access/session boundary previously exercised through the
        // browser socket, for both loopback address families without a listener.
        for address in ["127.0.0.1:13300", "[::1]:13300"] {
            let temp = temporary();
            let host = OwnerHost::open(temp.path(), address.parse().unwrap()).unwrap();
            let service = Service::new(host.clone().router());
            let exchange = salvo::test::TestClient::post(format!("http://{address}/console/session"))
                .add_header("host", address, true)
                .add_header("sec-fetch-site", "same-origin", true)
                .add_header("origin", format!("http://{address}"), true)
                .json(&serde_json::json!({"ticket":host.access_ticket().unwrap()}))
                .send(&service)
                .await;
            assert_eq!(exchange.status_code, Some(StatusCode::OK));
            let cookie = exchange.headers().get("set-cookie").unwrap()
                .to_str().unwrap().split(';').next().unwrap();
            let response = salvo::test::TestClient::get(format!("http://{address}/console/api/owned-agents"))
                .add_header("host", address, true)
                .add_header("sec-fetch-site", "same-origin", true)
                .add_header("cookie", cookie, true)
                .send(&service)
                .await;
            assert_eq!(response.status_code, Some(StatusCode::UNAUTHORIZED));
            host.console.retire();
            host.console.stop_owned_runtimes().await;
            host.console.stop_owner_provider().await;
        }
    }
}
