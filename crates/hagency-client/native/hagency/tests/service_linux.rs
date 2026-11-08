//! OwnerHost foreground supervisor contract. Historical custody/domain rows
//! are tested in SDK libraries and are never imported by this executable.
#[path = "owner_cli/mod.rs"]
mod owner;
use owner::*;
#[test]
fn native_service_unit_starts_and_reports_ready() {
    let root = hagency_platform::legacy_writer::test_support::initialized_tempdir().unwrap();
    let run = launch(&root.path().join("state"));
    assert_eq!(
        status(&request(run.address, "GET", "/ready", "", None)),
        200
    );
    assert_eq!(
        status(&request(run.address, "GET", "/console/login/", "", None)),
        404
    );
}
#[test]
fn native_service_unit_stops_cleanly_within_timeout_budget() {
    #[cfg(unix)]
    {
        let root = hagency_platform::legacy_writer::test_support::initialized_tempdir().unwrap();
        let mut run = launch(&root.path().join("state"));
        stop_clean(&mut run);
    }
    #[cfg(not(unix))]
    {
        let out = command().args(["open", "--no-open"]).output().unwrap();
        assert!(
            !out.status.success(),
            "The test host has no browser access command"
        );
    }
}
#[test]
fn native_service_unit_restart_preserves_owner_state() {
    let root = hagency_platform::legacy_writer::test_support::initialized_tempdir().unwrap();
    let state = root.path().join("state");
    let mut run = launch(&state);
    let original = marker(&state);
    #[cfg(unix)]
    stop_clean(&mut run);
    #[cfg(not(unix))]
    {
        run.child.kill().unwrap();
        run.child.wait().unwrap();
    }
    drop(run);
    let run = launch(&state);
    assert_eq!(marker(&state), original);
    assert_eq!(
        status(&request(
            run.address,
            "GET",
            "/console/api/owned-agents",
            "",
            None
        )),
        401,
        "restart cannot recover a Pasion session or authorize inference"
    );
    assert!(!state.join("domain.sqlite3").exists());
}
