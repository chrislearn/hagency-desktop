//! One git worktree per thread (board #74): a faithful port of the retained
//! `router/src/worktree.ts` `WorktreeManager`. Concurrent threads of one agent
//! get DISTINCT worktrees (branch `hagency/<agent>/<thread>`), bootstrap is
//! fail-closed (a `running|failed|complete` state file in the worktree's git
//! dir; a failed/dirty workspace stays quarantined until operator repair), the
//! bootstrap inherits ONLY an allowlisted environment (never backend
//! credentials), and preparation is containable + idempotent.
//!
//! This module is the router code the board names. It performs real `git
//! worktree add/remove` and a real bootstrap subprocess; every observable the
//! retained `router-core` "conservative worktree lifecycle" cases assert maps
//! here (see `native/hagency-execution/tests/worktree.rs`).

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The allowlist of environment variables a worktree bootstrap may inherit.
/// Kept verbatim from `router/src/worktree.ts:7-14`: PATH/HOME/shell/locale/
/// term/proxy/XDG/SSL — and NOTHING else, so a backend credential in the
/// parent environment never reaches the bootstrap subprocess.
const INHERITED_BOOTSTRAP_ENV_KEYS: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TMPDIR",
    "TMP",
    "TEMP",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TERM",
    "NO_COLOR",
    "FORCE_COLOR",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "NODE_EXTRA_CA_CERTS",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
];

#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct WorktreeError(pub String);

fn err(message: impl Into<String>) -> WorktreeError {
    WorktreeError(message.into())
}

/// `router/src/worktree.ts:24-30`.
#[derive(Clone, Debug)]
pub struct WorktreeSpec {
    pub repository_path: String,
    pub worktrees_dir: String,
    pub agent_id: String,
    pub thread_root_event_id: String,
    pub bootstrap: Option<Vec<String>>,
}

/// `router/src/worktree.ts:32-38`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub branch: String,
    pub safe_label: String,
    pub resource_id: String,
    pub created: bool,
}

/// `segment` (router/src/worktree.ts:44-47): lowercase, every run of bytes
/// outside `[a-z0-9._-]` becomes a single `-`, leading/trailing dashes trimmed,
/// capped at 48; empty collapses to the fallback.
fn segment(value: &str, fallback: &str) -> String {
    let mut out = String::new();
    for c in value.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    let sliced: String = trimmed.chars().take(48).collect();
    if sliced.is_empty() {
        fallback.to_string()
    } else {
        sliced
    }
}

fn short_digest(value: &str) -> String {
    hex(&Sha256::digest(value.as_bytes()))[..12].to_string()
}

fn resource_digest(value: &str) -> String {
    hex(&Sha256::digest(value.as_bytes()))[..32].to_string()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn realpath(path: &Path) -> Result<PathBuf, WorktreeError> {
    path.canonicalize()
        .map_err(|e| err(format!("canonicalize {}: {e}", path.display())))
}

/// Run `git <args>` in `cwd`, returning trimmed stdout. A non-zero exit is an
/// error carrying stderr (execFileSync parity).
fn git(cwd: &Path, args: &[&str]) -> Result<String, WorktreeError> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| err(format!("git spawn failed: {e}")))?;
    if !out.status.success() {
        return Err(err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// True when the command exits 0; false on non-zero; only errors on spawn
/// failure. Used for `show-ref --verify --quiet` (branch existence).
fn git_succeeds(cwd: &Path, args: &[&str]) -> Result<bool, WorktreeError> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| err(format!("git spawn failed: {e}")))?;
    Ok(out.status.success())
}

/// `ensureInside` (router/src/worktree.ts:61-66): the target must be a strict
/// child of the worktrees dir.
fn ensure_inside(parent: &Path, child: &Path) -> Result<(), WorktreeError> {
    let relative = child
        .strip_prefix(parent)
        .map_err(|_| err("worktree target must be a strict child of worktrees_dir"))?;
    if relative.as_os_str().is_empty() {
        return Err(err(
            "worktree target must be a strict child of worktrees_dir",
        ));
    }
    Ok(())
}

/// `identity` (router/src/worktree.ts:68-84).
fn identity(spec: &WorktreeSpec, repository_path: &Path, worktrees_dir: &Path) -> WorktreeInfo {
    let agent = segment(&spec.agent_id, "agent");
    let thread_seg = segment(&spec.thread_root_event_id, "thread");
    let thread = format!(
        "{}-{}",
        thread_seg.chars().take(28).collect::<String>(),
        short_digest(&spec.thread_root_event_id)
    );
    let branch = format!("hagency/{agent}/{thread}");
    let target = worktrees_dir.join(&agent).join(&thread);
    let safe_label = format!("{agent}/{thread}");
    let resource_id = format!(
        "worktree:{}",
        resource_digest(
            &serde_json::to_string(&[
                repository_path.to_string_lossy().into_owned(),
                worktrees_dir.to_string_lossy().into_owned(),
                spec.agent_id.clone(),
                spec.thread_root_event_id.clone(),
            ])
            .unwrap_or_default()
        )
    );
    WorktreeInfo {
        path: target,
        branch,
        safe_label,
        resource_id,
        created: false,
    }
}

/// `bootstrapEnv` (router/src/worktree.ts:16-22).
fn bootstrap_env() -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    for key in INHERITED_BOOTSTRAP_ENV_KEYS {
        if let Ok(value) = std::env::var(key) {
            env.insert((*key).to_string(), value);
        }
    }
    env
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BootstrapStatus {
    Running,
    Failed,
    Complete,
}

#[derive(Debug, Clone)]
struct BootstrapState {
    status: BootstrapStatus,
    digest: String,
}

fn bootstrap_state_path(worktree_path: &Path) -> Result<PathBuf, WorktreeError> {
    let raw_git_dir = git(worktree_path, &["rev-parse", "--git-dir"])?;
    let git_dir = realpath(&worktree_path.join(raw_git_dir))?;
    Ok(git_dir.join("hagency-bootstrap.json"))
}

/// `readBootstrapState` (router/src/worktree.ts:98-111): a missing file is
/// None; a present file that is not exactly `{version:1, status, digest}` is
/// also None (corrupt → operator repair, distinguished by the caller).
fn read_bootstrap_state(path: &Path) -> Option<BootstrapState> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let status = match value.get("status")?.as_str()? {
        "running" => BootstrapStatus::Running,
        "failed" => BootstrapStatus::Failed,
        "complete" => BootstrapStatus::Complete,
        _ => return None,
    };
    if value.get("version")?.as_u64()? != 1 {
        return None;
    }
    let digest = value.get("digest")?.as_str()?;
    if digest.is_empty() {
        return None;
    }
    Some(BootstrapState {
        status,
        digest: digest.to_string(),
    })
}

fn write_bootstrap_state(
    path: &Path,
    status: BootstrapStatus,
    digest: &str,
) -> Result<(), WorktreeError> {
    let status = match status {
        BootstrapStatus::Running => "running",
        BootstrapStatus::Failed => "failed",
        BootstrapStatus::Complete => "complete",
    };
    let body = format!(
        "{}\n",
        serde_json::json!({"version":1,"status":status,"digest":digest})
    );
    std::fs::write(path, body).map_err(|e| err(format!("write bootstrap state: {e}")))?;
    Ok(())
}

/// `ensureBootstrap` (router/src/worktree.ts:117-150): fail-closed. A corrupt
/// state file, or a dirty workspace with a failed/changed bootstrap, refuses
/// with "operator repair is required".
fn ensure_bootstrap(spec: &WorktreeSpec, worktree_path: &Path) -> Result<(), WorktreeError> {
    let Some(bootstrap) = &spec.bootstrap else {
        return Ok(());
    };
    if bootstrap.is_empty() {
        return Ok(());
    }
    let executable = &bootstrap[0];
    if executable.is_empty() {
        return Err(err("worktree bootstrap executable is empty"));
    }
    let state_path = bootstrap_state_path(worktree_path)?;
    let state_exists = state_path.exists();
    let state = read_bootstrap_state(&state_path);
    if state_exists && state.is_none() {
        return Err(err(
            "worktree bootstrap state is corrupt; operator repair is required",
        ));
    }
    let bootstrap_digest = resource_digest(&serde_json::to_string(bootstrap).unwrap_or_default());
    if let Some(state) = &state
        && state.status == BootstrapStatus::Complete
        && state.digest == bootstrap_digest
    {
        return Ok(());
    }
    if state.is_some() && !git(worktree_path, &["status", "--porcelain"])?.is_empty() {
        let reason = if state
            .as_ref()
            .is_some_and(|s| s.status != BootstrapStatus::Complete)
        {
            "worktree bootstrap previously failed and left a dirty workspace"
        } else {
            "worktree bootstrap configuration changed while the workspace is dirty"
        };
        return Err(err(format!("{reason}; operator repair is required")));
    }
    write_bootstrap_state(&state_path, BootstrapStatus::Running, &bootstrap_digest)?;
    let run = Command::new(executable)
        .args(&bootstrap[1..])
        .current_dir(worktree_path)
        .env_clear()
        .envs(bootstrap_env())
        .output()
        .map_err(|e| err(format!("bootstrap spawn failed: {e}")))?;
    if !run.status.success() {
        write_bootstrap_state(&state_path, BootstrapStatus::Failed, &bootstrap_digest)?;
        return Err(err(format!(
            "worktree bootstrap failed: {}",
            String::from_utf8_lossy(&run.stderr).trim()
        )));
    }
    write_bootstrap_state(&state_path, BootstrapStatus::Complete, &bootstrap_digest)
}

/// `registeredWorktrees` (router/src/worktree.ts:259-270): parse
/// `git worktree list --porcelain` into branch → path.
fn registered_worktrees(
    repository_path: &Path,
) -> Result<BTreeMap<String, PathBuf>, WorktreeError> {
    let output = git(repository_path, &["worktree", "list", "--porcelain"])?;
    let mut result = BTreeMap::new();
    let mut current: Option<PathBuf> = None;
    for line in output.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            current = Some(PathBuf::from(path));
        } else if let Some(branch) = line.strip_prefix("branch refs/heads/") {
            if let Some(path) = &current {
                result.insert(branch.to_string(), path.clone());
            }
        } else if line.trim().is_empty() {
            current = None;
        }
    }
    Ok(result)
}

/// `WorktreeManager` (router/src/worktree.ts:152-271).
pub struct WorktreeManager;

impl Default for WorktreeManager {
    fn default() -> Self {
        Self::new()
    }
}

impl WorktreeManager {
    pub fn new() -> Self {
        WorktreeManager
    }

    /// `ensureAsync` (router/src/worktree.ts:155-205): prepare a worktree off
    /// the async worker threads. The retained manager forks a worker process so
    /// a slow bootstrap never blocks the backend event loop; the native
    /// equivalent is `spawn_blocking` so the blocking `git`/bootstrap work runs
    /// on the blocking pool, never an async runtime thread.
    pub async fn ensure_async(&self, spec: WorktreeSpec) -> Result<WorktreeInfo, WorktreeError> {
        tokio::task::spawn_blocking(move || WorktreeManager::new().ensure(spec))
            .await
            .map_err(|join| WorktreeError(format!("worktree preparation task failed: {join}")))?
    }

    /// `ensure` (router/src/worktree.ts:207-239): idempotent, containable,
    /// distinct-branch-per-thread.
    pub fn ensure(&self, spec: WorktreeSpec) -> Result<WorktreeInfo, WorktreeError> {
        let repository_path = realpath(Path::new(&spec.repository_path))?;
        let repository_root = realpath(Path::new(&git(
            &repository_path,
            &["rev-parse", "--show-toplevel"],
        )?))?;
        if repository_root != repository_path {
            return Err(err("repository_path must be the git worktree root"));
        }
        let requested_worktrees_dir = PathBuf::from(&spec.worktrees_dir);
        std::fs::create_dir_all(&requested_worktrees_dir)
            .map_err(|e| err(format!("mkdir worktrees dir: {e}")))?;
        let worktrees_dir = realpath(&requested_worktrees_dir)?;
        let mut resolved = identity(&spec, &repository_path, &worktrees_dir);
        ensure_inside(&worktrees_dir, &resolved.path)?;
        let registered = registered_worktrees(&repository_path)?;
        if let Some(existing_path) = registered.get(&resolved.branch) {
            let existing_canonical = realpath(existing_path)?;
            let target_canonical = if resolved.path.exists() {
                realpath(&resolved.path)?
            } else {
                resolved.path.clone()
            };
            if existing_canonical != target_canonical {
                return Err(err(format!(
                    "branch {} is already checked out elsewhere",
                    resolved.branch
                )));
            }
            if !resolved.path.exists() {
                return Err(err("registered worktree path is missing"));
            }
            ensure_bootstrap(&spec, &existing_canonical)?;
            resolved.path = existing_canonical;
            resolved.created = false;
            return Ok(resolved);
        }
        if resolved.path.exists() {
            return Err(err("refusing to reuse an unregistered worktree directory"));
        }
        if let Some(parent) = resolved.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| err(format!("mkdir parent: {e}")))?;
        }
        let branch_ref = format!("refs/heads/{}", resolved.branch);
        let branch_exists = git_succeeds(
            &repository_path,
            &["show-ref", "--verify", "--quiet", &branch_ref],
        )?;
        if branch_exists {
            git(
                &repository_path,
                &[
                    "worktree",
                    "add",
                    resolved.path.to_str().unwrap_or_default(),
                    &resolved.branch,
                ],
            )?;
        } else {
            git(
                &repository_path,
                &[
                    "worktree",
                    "add",
                    "-b",
                    &resolved.branch,
                    resolved.path.to_str().unwrap_or_default(),
                    "HEAD",
                ],
            )?;
        }
        ensure_bootstrap(&spec, &resolved.path)?;
        resolved.path = realpath(&resolved.path)?;
        resolved.created = true;
        Ok(resolved)
    }

    /// `remove` (router/src/worktree.ts:246-257): dirty without force refuses.
    pub fn remove(&self, spec: WorktreeSpec, force: bool) -> Result<WorktreeInfo, WorktreeError> {
        let repository_path = realpath(Path::new(&spec.repository_path))?;
        let worktrees_dir = realpath(Path::new(&spec.worktrees_dir))?;
        let resolved = identity(&spec, &repository_path, &worktrees_dir);
        ensure_inside(&worktrees_dir, &resolved.path)?;
        if !resolved.path.exists() {
            return Err(err("worktree does not exist"));
        }
        let actual = realpath(&resolved.path)?;
        let dirty = !git(&actual, &["status", "--porcelain"])?.is_empty();
        if dirty && !force {
            return Err(err("dirty worktree requires explicit force"));
        }
        let mut args = vec!["worktree", "remove"];
        if force {
            args.push("--force");
        }
        args.push(actual.to_str().unwrap_or_default());
        git(&repository_path, &args)?;
        Ok(WorktreeInfo {
            path: actual,
            created: false,
            ..resolved
        })
    }
}

// `inspect` returns a dirtiness fact that is not part of `WorktreeInfo`'s
// serialized identity; keep it out of the public struct and surface it through
// a dedicated accessor instead of a struct field. The retained manager's
// `WorktreeInspection` carries `dirty`; mirror that as a separate type.
impl WorktreeManager {
    /// `inspect` returning the retained `WorktreeInspection { ..info, dirty }`.
    pub fn inspect_with_dirty(
        &self,
        spec: WorktreeSpec,
    ) -> Result<(WorktreeInfo, bool), WorktreeError> {
        let info = self.ensure(spec)?;
        let dirty = !git(&info.path, &["status", "--porcelain"])?.is_empty();
        Ok((info, dirty))
    }
}
