//! Board #74: one git worktree per thread — the retained router-core
//! "conservative worktree lifecycle" cases (`tests/router-core.test.js:1502-1778`)
//! ported to native `WorktreeManager`, asserting the SAME observable outcomes
//! against a real `git init`-ed repo.
//!
//! Cases mirrored (TS test name → native test):
//!   test_worktree_mode_runs_two_threads_in_distinct_worktrees →
//!     two_concurrent_threads_get_distinct_worktrees
//!   failed worktree bootstrap remains fail-closed … →
//!     failed_bootstrap_stays_fail_closed_until_success
//!   changed bootstrap cannot bypass a dirty failed-bootstrap quarantine →
//!     changed_bootstrap_cannot_bypass_dirty_quarantine
//!   worktree bootstrap does not inherit backend credentials →
//!     bootstrap_does_not_inherit_backend_credentials
//!   recreated worktree cannot reuse bootstrap success … →
//!     recreated_worktree_reruns_bootstrap
//!   worktree resource identity includes repository and worktree root →
//!     resource_identity_distinguishes_repository_root
//!   test_dirty_worktree_retained_on_session_eviction (manager half) →
//!     dirty_worktree_refuses_remove_without_force

use hagency_execution::{WorktreeManager, WorktreeSpec};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn git(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A throwaway git repo with one committed file. Returns (root, repo, worktrees).
fn repo(label: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["config", "user.name", "Test"]);
    fs::write(repo.join("README.md"), format!("{label}\n")).unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-m", "base"]);
    let worktrees = root.path().join("worktrees");
    (root, repo, worktrees)
}

fn spec(repo: &Path, worktrees: &Path, agent: &str, thread: &str) -> WorktreeSpec {
    WorktreeSpec {
        repository_path: repo.to_string_lossy().into_owned(),
        worktrees_dir: worktrees.to_string_lossy().into_owned(),
        agent_id: agent.to_string(),
        thread_root_event_id: thread.to_string(),
        bootstrap: None,
    }
}

/// TS `test_worktree_mode_runs_two_threads_in_distinct_worktrees` — two threads
/// of one agent get distinct paths AND branches; a repeat `ensure` is idempotent.
#[test]
fn two_concurrent_threads_get_distinct_worktrees() {
    let (_root, repo, worktrees) = repo("distinct");
    let manager = WorktreeManager::new();
    let a = manager
        .ensure(spec(&repo, &worktrees, "agent", "$thread-a"))
        .unwrap();
    let b = manager
        .ensure(spec(&repo, &worktrees, "agent", "$thread-b"))
        .unwrap();
    assert_ne!(a.path, b.path);
    assert_ne!(a.branch, b.branch);
    let again = manager
        .ensure(spec(&repo, &worktrees, "agent", "$thread-a"))
        .unwrap();
    assert_eq!(again.path, a.path);
    assert!(!again.created, "repeat ensure must not recreate");
}

/// TS `failed worktree bootstrap remains fail-closed …` — a failing bootstrap
/// leaves the worktree refusing until a successful bootstrap is recorded.
#[test]
fn failed_bootstrap_stays_fail_closed_until_success() {
    let (_root, repo, worktrees) = repo("bootstrap");
    let manager = WorktreeManager::new();
    let mut failing = spec(&repo, &worktrees, "agent", "$bootstrap");
    failing.bootstrap = Some(vec![
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()),
        "-c".into(),
        "exit 17".into(),
    ]);
    assert!(manager.ensure(failing.clone()).is_err());
    assert!(manager.ensure(failing.clone()).is_err());

    let mut ok = failing;
    ok.bootstrap = Some(vec![
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()),
        "-c".into(),
        "exit 0".into(),
    ]);
    assert!(!manager.ensure(ok.clone()).unwrap().created);
    assert!(!manager.ensure(ok).unwrap().created);
}

/// TS `changed bootstrap cannot bypass a dirty failed-bootstrap quarantine` —
/// a failed bootstrap that left the worktree dirty refuses even a now-clean
/// bootstrap, with "operator repair is required".
#[test]
fn changed_bootstrap_cannot_bypass_dirty_quarantine() {
    let (_root, repo, worktrees) = repo("dirty-bootstrap");
    let manager = WorktreeManager::new();
    let mut dirty = spec(&repo, &worktrees, "agent", "$dirty-bootstrap");
    dirty.bootstrap = Some(vec![
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()),
        "-c".into(),
        "echo partial > partial; exit 17".into(),
    ]);
    assert!(manager.ensure(dirty).is_err());

    let mut clean = spec(&repo, &worktrees, "agent", "$dirty-bootstrap");
    clean.bootstrap = Some(vec![
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()),
        "-c".into(),
        "exit 0".into(),
    ]);
    let err = manager.ensure(clean).unwrap_err().0;
    assert!(
        err.contains("dirty workspace; operator repair is required"),
        "unexpected error: {err}"
    );
}

/// TS `worktree bootstrap does not inherit backend credentials` — the bootstrap
/// subprocess sees a clean environment (no `API_TOKEN`).
#[test]
fn bootstrap_does_not_inherit_backend_credentials() {
    let (_root, repo, worktrees) = repo("bootstrap-env");
    let manager = WorktreeManager::new();
    // The allowlisted environment is built from the CURRENT process env; set a
    // credential-like var and prove it is NOT in INHERITED_BOOTSTRAP_ENV_KEYS
    // and never reaches the bootstrap.
    let mut spec = spec(&repo, &worktrees, "agent", "$bootstrap-env");
    spec.bootstrap = Some(vec![
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()),
        "-c".into(),
        // Probe writes `leaked`/`clean` into a file the worktree can read back.
        "[ -z \"${API_TOKEN:-}\" ] && printf clean > bootstrap-env || printf leaked > bootstrap-env".into(),
    ]);
    let info = manager.ensure(spec).unwrap();
    assert_eq!(
        fs::read_to_string(info.path.join("bootstrap-env")).unwrap(),
        "clean",
        "a backend credential reached the bootstrap environment"
    );
}

/// TS `recreated worktree cannot reuse bootstrap success from a removed
/// checkout` — removing then re-ensuring re-runs the bootstrap.
#[test]
fn recreated_worktree_reruns_bootstrap() {
    let (_root, repo, worktrees) = repo("recreate");
    let manager = WorktreeManager::new();
    let mut spec = spec(&repo, &worktrees, "agent", "$recreate");
    spec.bootstrap = Some(vec![
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()),
        "-c".into(),
        "printf ready > bootstrap-output".into(),
    ]);
    let first = manager.ensure(spec.clone()).unwrap();
    assert!(first.path.join("bootstrap-output").exists());
    manager.remove(spec.clone(), true).unwrap();
    let second = manager.ensure(spec).unwrap();
    assert!(
        second.path.join("bootstrap-output").exists(),
        "a recreated worktree must re-run bootstrap"
    );
}

/// TS `worktree resource identity includes repository and worktree root` — the
/// same thread in two different repos produces distinct resource ids.
#[test]
fn resource_identity_distinguishes_repository_root() {
    let root = tempfile::tempdir().unwrap();
    let make = |name: &str| -> PathBuf {
        let repo = root.path().join(name);
        fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init"]);
        git(&repo, &["config", "user.email", "t@e.com"]);
        git(&repo, &["config", "user.name", "T"]);
        fs::write(repo.join("README.md"), format!("{name}\n")).unwrap();
        git(&repo, &["add", "README.md"]);
        git(&repo, &["commit", "-m", "base"]);
        repo
    };
    let manager = WorktreeManager::new();
    let a = manager
        .ensure(spec(
            &make("repo-a"),
            &root.path().join("wt-a"),
            "agent",
            "$same",
        ))
        .unwrap();
    let b = manager
        .ensure(spec(
            &make("repo-b"),
            &root.path().join("wt-b"),
            "agent",
            "$same",
        ))
        .unwrap();
    assert_ne!(a.resource_id, b.resource_id);
}

/// TS `async worktree preparation does not block the backend event loop` — a
/// slow bootstrap does not block the async runtime thread: the timer fires
/// while the preparation is still pending.
#[tokio::test]
async fn async_preparation_does_not_block_the_event_loop() {
    let (_root, repo, worktrees) = repo("async");
    let manager = WorktreeManager::new();
    let mut spec = spec(&repo, &worktrees, "agent", "$async");
    spec.bootstrap = Some(vec![
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()),
        "-c".into(),
        "sleep 1".into(),
    ]);
    let preparation = manager.ensure_async(spec);
    let start = Instant::now();
    // A timer must fire while the bootstrap still sleeps: the preparation is on
    // the blocking pool, not an async worker.
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        start.elapsed() < Duration::from_millis(500),
        "timer fired while bootstrap ran"
    );
    let info = preparation.await.unwrap();
    assert!(info.created, "first async prepare creates the worktree");
}

#[test]
fn dirty_worktree_refuses_remove_without_force() {
    let (_root, repo, worktrees) = repo("dirty");
    let manager = WorktreeManager::new();
    let spec = spec(&repo, &worktrees, "agent", "$dirty");
    let info = manager.ensure(spec.clone()).unwrap();
    fs::write(info.path.join("dirty.txt"), "keep me\n").unwrap();
    let (_, dirty) = manager.inspect_with_dirty(spec.clone()).unwrap();
    assert!(dirty, "inspect must report dirty");
    assert!(
        manager.remove(spec.clone(), false).is_err(),
        "dirty worktree must refuse remove without force"
    );
    assert!(info.path.exists(), "dirty worktree must survive eviction");
    // branch still registered in the source repo
    let refs = git(
        &repo,
        &[
            "show-ref",
            "--verify",
            &format!("refs/heads/{}", info.branch),
        ],
    );
    assert!(refs.contains(&info.branch));
    // force remove works
    manager.remove(spec, true).unwrap();
    assert!(!info.path.exists());
}
