use super::*;
fn input(f: &Fixture, id: &str) -> ApprovalVerdictInput {
    ApprovalVerdictInput {
        target: f.db.approval_intake_target(id, 1011).unwrap(),
        source_digest: "a".repeat(64),
        verdict: f.verdict(id, ApprovalChoice::Always, "typed_owner"),
    }
}
#[test]
fn native_matrix_approval_recovery_exact_historical_source_after_retirement() {
    let mut f = Fixture::new(true);
    let a = f.admit(0, 1);
    let v = input(&f, &a.id);
    assert!(f.db.approval_verdict_receipt(&v).unwrap().is_none());
    assert_eq!(
        f.db.admit_approval_verdict(&v, 1012).unwrap().state,
        "decided"
    );
    assert_eq!(f.grants(0).len(), 1);
    let authority = f.db.approval_room_authority(&f.agents[0]).unwrap();
    let prior = f.db.approval_room_capture(&authority).unwrap();
    f.db.fence_approval_room(&authority, "BOT_DEVICE", 1, prior.as_ref())
        .unwrap();
    assert!(f.grants(0)[0].revoked);
    assert!(f.db.admit_approval_verdict(&v, 20_000).is_err());
    assert!(f.db.approval_verdict_receipt(&v).unwrap().is_some());
    let mut changed = v.clone();
    changed.source_digest = "b".repeat(64);
    assert!(matches!(
        f.db.approval_verdict_receipt(&changed),
        Err(Error::Conflict)
    ));
    changed = v.clone();
    changed.verdict.choice = ApprovalChoice::Deny;
    assert!(matches!(
        f.db.approval_verdict_receipt(&changed),
        Err(Error::Conflict)
    ));
    assert_eq!(f.grants(0).len(), 1);
}
#[test]
fn native_matrix_approval_scope_changed_target_and_old_negative_cannot_override_new_room() {
    let mut f = Fixture::new(true);
    let a = f.admit(0, 1);
    let mut v = input(&f, &a.id);
    v.target.device_id = "OTHER".into();
    assert!(f.db.admit_approval_verdict(&v, 1012).is_err());
    assert!(f.grants(0).is_empty());
    let authority = f.db.approval_room_authority(&f.agents[0]).unwrap();
    let prior = f.db.approval_room_capture(&authority).unwrap();
    let mut next = f.rooms[0].clone();
    next.generation = 2;
    next.device_id = "NEW_DEVICE".into();
    f.db.observe_approval_room(&next, 1013).unwrap();
    let newer = f.db.approval_room_capture(&authority).unwrap().unwrap();
    f.db.fence_approval_room(&authority, "BOT_DEVICE", 1, prior.as_ref())
        .unwrap();
    assert_eq!(
        f.db.approval_room_capture(&authority)
            .unwrap()
            .unwrap()
            .digest,
        newer.digest
    );
    let available: bool = f
        .sql()
        .query_row("SELECT available FROM approval_rooms", [], |r| r.get(0))
        .unwrap();
    assert!(available);
}
#[test]
fn native_matrix_approval_identity_candidate_fence_covers_lost_positive_without_agent_rotation() {
    let mut f = Fixture::new(true);
    let authority = f.db.approval_room_authority(&f.agents[0]).unwrap();
    let transport = f.db.matrix_transport_state(&f.agents[0]).unwrap().unwrap();
    f.db.fence_approval_room(&authority, "BOT_DEVICE", 1, None)
        .unwrap();
    assert!(
        f.db.matrix_transport_state(&f.agents[0])
            .unwrap()
            .unwrap()
            .observation
            == transport.observation
    );
    assert!(
        f.db.matrix_transport_state(&f.agents[0])
            .unwrap()
            .unwrap()
            .available
    );
    // ADR-183 C: the fence was written over the intact room state, so the
    // same-generation positive observation heals the row (the reversal of
    // "restoration still needs a new generation").
    assert!(
        f.db.request_owner_approval(&f.caps[0], &f.input(0, 1), 1012)
            .is_err()
    );
    f.db.observe_approval_room(&f.rooms[0], 1013).unwrap();
    assert!(
        f.db.approval_room_capture(&authority)
            .unwrap()
            .unwrap()
            .available
    );
    f.db.request_owner_approval(&f.caps[0], &f.input(0, 1), 1014)
        .unwrap();
}
/// ADR-183 C: `available=0` is the fence word, and a good observation at the
/// same generation with the same members, privacy, encryption and bot device
/// restores it; the startup refresh is exactly that observation. A safe
/// snapshot that differs at the same generation is still Conflict, a room
/// whose state really changed is the next generation's matter, and the grants
/// the fence revoked stay revoked.
#[test]
fn native_fenced_approval_room_heals_on_a_good_observation() {
    let mut f = Fixture::new(true);
    let a = f.admit(0, 1);
    f.choose(&a.id, ApprovalChoice::Always);
    assert!(!f.grants(0)[0].revoked);
    let authority = f.db.approval_room_authority(&f.agents[0]).unwrap();
    let before = f.db.approval_room_capture(&authority).unwrap().unwrap();
    // Given an approval room fenced by an earlier refusal.
    f.db.fence_approval_room(&authority, "BOT_DEVICE", 1, None)
        .unwrap();
    assert!(
        !f.db
            .approval_room_capture(&authority)
            .unwrap()
            .unwrap()
            .available
    );
    assert!(f.grants(0)[0].revoked);
    assert!(
        f.db.request_owner_approval(&f.caps[0], &f.input(0, 2), 1013)
            .is_err()
    );
    // When observed again at the same generation with the same snapshot.
    f.db.observe_approval_room(&f.rooms[0], 1014).unwrap();
    // Then the row is available again at the same generation, and the next
    // startup (which observes exactly this) admits it.
    let healed = f.db.approval_room_capture(&authority).unwrap().unwrap();
    assert!(healed.available);
    assert_eq!(healed.generation, 1);
    assert_eq!(healed.device_id, before.device_id);
    assert_eq!(healed.digest, before.digest);
    f.db.request_owner_approval(&f.caps[0], &f.input(0, 2), 1015)
        .unwrap();
    assert!(
        f.grants(0)[0].revoked,
        "healing restores availability, not grants"
    );
    // And an observation whose safe snapshot differs at the same generation
    // (another bot device) stays fenced with Conflict.
    f.db.fence_approval_room(&authority, "BOT_DEVICE", 1, None)
        .unwrap();
    let mut other_device = f.rooms[0].clone();
    other_device.device_id = "OTHER_DEVICE".into();
    assert!(matches!(
        f.db.observe_approval_room(&other_device, 1016),
        Err(Error::Conflict)
    ));
    assert!(
        !f.db
            .approval_room_capture(&authority)
            .unwrap()
            .unwrap()
            .available
    );
    // A fence written by negative room evidence (a third member) is not
    // healed by the original snapshot: that snapshot differs from the one the
    // fence was written over, so it is still Conflict at the same generation.
    f.db.observe_approval_room(&f.rooms[0], 1017).unwrap();
    let mut third = f.rooms[0].clone();
    third.joined.insert("@third:example.test".into());
    f.db.observe_approval_room(&third, 1018).unwrap();
    assert!(matches!(
        f.db.observe_approval_room(&f.rooms[0], 1019),
        Err(Error::Conflict)
    ));
    let mut next = f.rooms[0].clone();
    next.generation = 2;
    f.db.observe_approval_room(&next, 1020).unwrap();
    assert!(
        f.db.approval_room_capture(&authority)
            .unwrap()
            .unwrap()
            .available
    );
}
#[test]
fn native_matrix_approval_scope_expiry_exact_private_owner_and_target_are_rechecked_atomically() {
    let mut f = Fixture::new(true);
    let mut req = f.input(0, 1);
    req.expires_at = 1012;
    let a = f.db.request_owner_approval(&f.caps[0], &req, 1010).unwrap();
    let v = input(&f, &a.id);
    for mutation in 0..4 {
        let mut changed = v.clone();
        match mutation {
            0 => changed.verdict.sender_mxid = "@other:example.test".into(),
            1 => changed.verdict.room_id = "!project:example.test".into(),
            2 => changed.target.authority.engagement_id = f.agents[1].clone(),
            3 => changed.target.binding_generation += 1,
            _ => unreachable!(),
        }
        assert!(f.db.admit_approval_verdict(&changed, 1011).is_err());
    }
    assert!(f.db.admit_approval_verdict(&v, 1012).is_err());
    assert!(f.db.approval_verdict_receipt(&v).unwrap().is_none());
    assert!(f.grants(0).is_empty());
}
#[test]
fn native_matrix_approval_recovery_sqlite_rollback_keeps_grant_and_source_atomic() {
    let mut f = Fixture::new(true);
    let a = f.admit(0, 1);
    let v = input(&f, &a.id);
    let sql = f.sql();
    sql.execute_batch("CREATE TRIGGER fail_typed_verdict AFTER INSERT ON approval_verdict_receipts BEGIN SELECT RAISE(ABORT,'fixture rollback'); END;").unwrap();
    assert!(f.db.admit_approval_verdict(&v, 1012).is_err());
    assert!(f.db.approval_verdict_receipt(&v).unwrap().is_none());
    assert!(f.grants(0).is_empty());
    assert_eq!(f.db.approval_summary(&a.id).unwrap().state, "pending");
    sql.execute_batch("DROP TRIGGER fail_typed_verdict;")
        .unwrap();
    f.db.admit_approval_verdict(&v, 1013).unwrap();
    assert!(f.db.approval_verdict_receipt(&v).unwrap().is_some());
    assert_eq!(f.grants(0).len(), 1);
}
