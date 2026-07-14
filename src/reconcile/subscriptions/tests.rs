use super::*;
use trellis_core::ResourceCommandCause;
use trellis_testing::ResourceLedger;

fn set<const N: usize>(items: [&str; N]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn sessions<const N: usize>(
    e: [(&str, BTreeSet<String>); N],
) -> BTreeMap<String, BTreeSet<String>> {
    e.into_iter().map(|(id, c)| (id.to_string(), c)).collect()
}

fn open_ids(effects: &[SubEffect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|e| match e {
            SubEffect::Open { id, .. } => Some(id.to_string()),
            _ => None,
        })
        .collect()
}

fn close_ids(effects: &[SubEffect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|e| match e {
            SubEffect::Close { id } => Some(id.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn opens_one_narrow_req_per_entity_and_is_idempotent() {
    let mut r = SubscriptionReconciler::new().unwrap();
    let mut ledger = ResourceLedger::new();

    let snap = CoverageSnapshot {
        daemon_channels: set(["room-a", "room-b"]),
        addressed_pubkeys: set(["pk-1", "pk-2"]),
        archived_channels: BTreeSet::new(),
        sessions: BTreeMap::new(),
    };
    let (effects, result) = r.sync(&snap).unwrap();
    ledger.apply_result(&result);
    r.assert_oracle().unwrap();

    // Exactly one daemon-global kind:9000 discovery REQ, one `#h` + one `#d`
    // per channel, and one `#p` per pubkey — never a kind:0 firehose.
    let opened: BTreeSet<String> = open_ids(&effects).into_iter().collect();
    assert_eq!(
        opened,
        set([
            "te-v2-global-kind-9000",
            "te-v2-h-room-a",
            "te-v2-h-room-b",
            "te-v2-gstate-room-a",
            "te-v2-gstate-room-b",
            "te-v2-p-pk-1",
            "te-v2-p-pk-2",
        ])
    );
    for e in &effects {
        if let SubEffect::Open { filter, .. } = e {
            let json = serde_json::to_string(filter).unwrap();
            assert!(!json.contains("\"kinds\":[0"), "no kind:0 firehose: {json}");
        }
    }

    // Re-syncing the identical coverage is a no-op: no new opens, no closes.
    let (again, result2) = r.sync(&snap).unwrap();
    ledger.apply_result(&result2);
    r.assert_oracle().unwrap();
    assert!(
        again.is_empty(),
        "idempotent resync emits nothing: {again:?}"
    );

    ledger.assert_all_resources_have_owner().unwrap();
    ledger.assert_no_duplicate_close().unwrap();
}

/// The refcount test the aggregate registry never had: a channel two sessions
/// share is closed ONLY when the LAST session leaves it, never while another
/// still holds it. The graph's `owner_count` is the authoritative refcount and
/// the emitted effects are the wire truth.
#[test]
fn channel_closes_only_when_last_session_leaves() {
    let mut r = SubscriptionReconciler::new().unwrap();
    let shared_h = sub_key(Space::ChannelH, "shared");

    // Two live sessions both joined "shared"; s1 also sits in "solo".
    let (effects, _result) = r
        .sync(&CoverageSnapshot {
            sessions: sessions([("s1", set(["shared", "solo"])), ("s2", set(["shared"]))]),
            ..Default::default()
        })
        .unwrap();
    r.assert_oracle().unwrap();

    // "shared" is opened exactly once despite two owners (refcounted, coalesced).
    let opened = open_ids(&effects);
    assert_eq!(
        opened.iter().filter(|id| *id == "te-v2-h-shared").count(),
        1,
        "shared #h opened once for two owners: {opened:?}"
    );
    assert_eq!(r.owner_count(&shared_h), 2, "two sessions own shared #h");

    // s1 LEAVES "shared" (still alive in "solo"); s2 still holds it → NO close.
    let (after_s1, _result) = r
        .sync(&CoverageSnapshot {
            sessions: sessions([("s1", set(["solo"])), ("s2", set(["shared"]))]),
            ..Default::default()
        })
        .unwrap();
    r.assert_oracle().unwrap();
    assert!(
        !close_ids(&after_s1).contains(&"te-v2-h-shared".to_string()),
        "shared must NOT close while s2 still holds it: {after_s1:?}"
    );
    assert_eq!(r.owner_count(&shared_h), 1, "only s2 still owns shared #h");
    assert!(r.covers_channel("shared"), "shared is still covered by s2");

    // s2 LEAVES "shared" — now the last owner is gone → a REAL close.
    let (after_s2, _result) = r
        .sync(&CoverageSnapshot {
            sessions: sessions([("s1", set(["solo"])), ("s2", BTreeSet::new())]),
            ..Default::default()
        })
        .unwrap();
    r.assert_oracle().unwrap();
    let closed = close_ids(&after_s2);
    assert!(
        closed.contains(&"te-v2-h-shared".to_string())
            && closed.contains(&"te-v2-gstate-shared".to_string()),
        "last leave emits a real CLOSE for shared #h and #d: {after_s2:?}"
    );
    assert_eq!(
        r.owner_count(&shared_h),
        0,
        "no owner remains for shared #h"
    );
    assert!(!r.covers_channel("shared"), "shared is torn down");
    // "solo" is untouched throughout — s1 never left it.
    assert!(r.covers_channel("solo"));

    // The audit names that a command (the close) was last emitted for shared #h.
    let why = r
        .why_command(&shared_h)
        .expect("a command was emitted for shared #h");
    assert_eq!(why.kind, ResourceCommandKind::Close);
}

/// A single owner join → leave → rejoin cycle: every open and close is a real
/// emitted command, so the `ResourceLedger` mirrors the wire exactly. Asserts no
/// duplicate closes and no orphaned opens across the cycle.
#[test]
fn join_leave_rejoin_ledger_is_clean() {
    let mut r = SubscriptionReconciler::new().unwrap();
    let mut ledger = ResourceLedger::new();
    let room_h = sub_key(Space::ChannelH, "room");

    let step = |r: &mut SubscriptionReconciler, chans: BTreeSet<String>| {
        r.sync(&CoverageSnapshot {
            sessions: sessions([("s1", chans)]),
            ..Default::default()
        })
        .unwrap()
        .1
    };

    // join
    let result = step(&mut r, set(["room"]));
    ledger.apply_result(&result);
    r.assert_oracle().unwrap();
    assert_eq!(r.owner_count(&room_h), 1);

    // leave (sole owner → real CLOSE)
    let result = step(&mut r, BTreeSet::new());
    ledger.apply_result(&result);
    r.assert_oracle().unwrap();
    assert_eq!(r.owner_count(&room_h), 0);
    ledger.assert_resource_not_open(&room_h).unwrap();

    // rejoin (real OPEN again)
    let result = step(&mut r, set(["room"]));
    ledger.apply_result(&result);
    r.assert_oracle().unwrap();
    assert_eq!(r.owner_count(&room_h), 1);
    assert!(r.covers_channel("room"));

    ledger.assert_all_resources_have_owner().unwrap();
    ledger.assert_no_duplicate_close().unwrap();
}

/// Ending a session closes its scope, tearing down exactly the REQs it solely
/// owned — and the audit names the ScopeClosed cause.
#[test]
fn session_end_scope_close_tears_down_sole_owned_reqs() {
    let mut r = SubscriptionReconciler::new().unwrap();
    let mut ledger = ResourceLedger::new();

    let (_open, result) = r
        .sync(&CoverageSnapshot {
            sessions: sessions([("s1", set(["room"]))]),
            ..Default::default()
        })
        .unwrap();
    ledger.apply_result(&result);
    r.assert_oracle().unwrap();
    assert!(r.covers_channel("room"));

    // s1 ends entirely (gone from the snapshot) → its scope is closed.
    let (effects, result) = r.sync(&CoverageSnapshot::default()).unwrap();
    ledger.apply_result(&result);
    r.assert_oracle().unwrap();

    let closed = close_ids(&effects);
    assert!(
        closed.contains(&"te-v2-h-room".to_string())
            && closed.contains(&"te-v2-gstate-room".to_string()),
        "session-end tears down its solely-owned REQs: {effects:?}"
    );
    let why = r
        .why_command(&sub_key(Space::ChannelH, "room"))
        .expect("a close was emitted for room #h");
    assert!(
        matches!(why.cause, ResourceCommandCause::ScopeClosed { .. }),
        "close cause names the session scope teardown: {:?}",
        why.cause
    );
    ledger
        .assert_resource_not_open(&sub_key(Space::ChannelH, "room"))
        .unwrap();
    ledger.assert_all_resources_have_owner().unwrap();
    ledger.assert_no_duplicate_close().unwrap();
}

/// A channel held by BOTH the daemon scope (membership) and a session scope
/// survives the session leaving — the daemon owner keeps it open.
#[test]
fn daemon_owned_channel_survives_session_leave() {
    let mut r = SubscriptionReconciler::new().unwrap();

    let (_open, _r) = r
        .sync(&CoverageSnapshot {
            daemon_channels: set(["managed"]),
            sessions: sessions([("s1", set(["managed"]))]),
            ..Default::default()
        })
        .unwrap();
    r.assert_oracle().unwrap();

    // Session s1 ends; the daemon scope still owns "managed" → no close.
    let (effects, _r) = r
        .sync(&CoverageSnapshot {
            daemon_channels: set(["managed"]),
            ..Default::default()
        })
        .unwrap();
    r.assert_oracle().unwrap();
    assert!(
        close_ids(&effects).is_empty(),
        "daemon-owned channel must not close on session leave: {effects:?}"
    );
    assert!(r.covers_channel("managed"));

    // Only when the daemon also drops it (e.g. membership revoked) does it close.
    let (final_effects, _r) = r.sync(&CoverageSnapshot::default()).unwrap();
    r.assert_oracle().unwrap();
    assert!(close_ids(&final_effects).contains(&"te-v2-h-managed".to_string()));
    assert!(!r.covers_channel("managed"));
}

/// Archived channels are excluded from all coverage even when a session lists
/// them as joined.
#[test]
fn archived_channels_are_excluded() {
    let mut r = SubscriptionReconciler::new().unwrap();
    let (effects, _r) = r
        .sync(&CoverageSnapshot {
            daemon_channels: set(["live", "old"]),
            archived_channels: set(["old"]),
            sessions: sessions([("s1", set(["live", "old"]))]),
            ..Default::default()
        })
        .unwrap();
    r.assert_oracle().unwrap();
    let opened = open_ids(&effects);
    assert!(opened.contains(&"te-v2-h-live".to_string()));
    assert!(
        !opened.iter().any(|id| id.contains("old")),
        "archived channel opens nothing: {opened:?}"
    );
    assert!(r.covers_channel("live"));
    assert!(!r.covers_channel("old"));
}
