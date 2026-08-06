use super::*;

fn state_for(s: &Store, event_id: &str, target_pubkey: &str) -> String {
    s.conn
        .query_row(
            "SELECT state FROM inbox WHERE event_id=?1 AND target_pubkey=?2",
            params![event_id, target_pubkey],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn inbox_event_prefix_lookup_can_filter_target_pubkey() {
    let s = Store::open_memory().unwrap();
    s.enqueue_inbox("evt-abc", "pk-1", "pk", "room", "one", 10)
        .unwrap();
    s.enqueue_inbox("evt-abc", "pk-2", "pk", "room", "two", 11)
        .unwrap();
    s.enqueue_inbox("evt-other", "pk-1", "pk", "room", "three", 12)
        .unwrap();

    let rows = s.inbox_by_event_prefix("evt-a").unwrap();
    assert_eq!(rows.len(), 2);

    let row = s.inbox_by_event_prefix_and_target("evt-a", "pk-2").unwrap();
    assert_eq!(row.len(), 1);
    assert_eq!(row[0].body, "two");
}

#[test]
fn claim_pending_event_ids_claims_only_the_planned_rows() {
    let s = Store::open_memory().unwrap();
    upsert_runtime(&s, "pk-1", 1);
    insert_chat(&s, "evt-1", 10);
    insert_chat(&s, "evt-2", 11);
    s.enqueue_inbox("evt-1", "pk-1", "pk", "room", "one", 10)
        .unwrap();
    s.enqueue_inbox("evt-2", "pk-1", "pk", "room", "two", 11)
        .unwrap();

    let rows = s
        .claim_pending_event_ids_for_pubkey(&["evt-2".into()], "pk-1", 20)
        .unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].event_id, "evt-2");
    assert_eq!(state_for(&s, "evt-1", "pk-1"), "pending");
    assert_eq!(state_for(&s, "evt-2", "pk-1"), "delivered");
    assert_eq!(
        s.peek_pending_for_pubkey("pk-1").unwrap()[0].event_id,
        "evt-1"
    );
}

#[test]
fn pending_event_survives_runtime_replacement() {
    let s = Store::open_memory().unwrap();
    upsert_runtime(&s, "pk-agent", 10);
    insert_chat(&s, "evt", 11);
    s.enqueue_inbox("evt", "pk-agent", "sender", "room", "hello", 11)
        .unwrap();
    s.mark_runtime_stopped("pk-agent", StopReason::Unknown, 11)
        .unwrap();
    upsert_runtime(&s, "pk-agent", 12);

    let replacement = s.get_session("pk-agent").unwrap().unwrap();
    assert_eq!(replacement.pubkey, "pk-agent");
    let claimed = s.claim_pending_for_pubkey(&replacement.pubkey, 13).unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].event_id, "evt");
}

#[test]
fn offline_inbox_rows_join_attachment_directory_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    {
        let store = Store::open(&path).unwrap();
        store
            .record_message(&RecordMessage {
                message_id: "evt-files".into(),
                thread_id: "room".into(),
                channel_h: "room".into(),
                author_pubkey: "human".into(),
                body: "see [report.md]".into(),
                created_at: 10,
                sync_state: "accepted".into(),
                native_event_id: Some("evt-files".into()),
                error: None,
            })
            .unwrap();
        store
            .set_message_attachment_dir("evt-files", Path::new("/tmp/mosaico-files/evt-fi"))
            .unwrap();
        store
            .enqueue_inbox(
                "evt-files",
                "pk-agent",
                "human",
                "room",
                "see [report.md]",
                10,
            )
            .unwrap();
    }

    let reopened = Store::open(&path).unwrap();
    let rows = reopened.peek_pending_for_pubkey("pk-agent").unwrap();
    assert_eq!(rows[0].attachment_dir, "/tmp/mosaico-files/evt-fi");
}

#[test]
fn hook_claim_stages_one_work_start_reaction() {
    let s = Store::open_memory().unwrap();
    upsert_runtime(&s, "pk", 1);
    insert_chat(&s, "evt", 10);
    s.enqueue_inbox("evt", "pk", "human", "room", "start", 10)
        .unwrap();

    assert_eq!(s.claim_pending_for_pubkey("pk", 11).unwrap().len(), 1);
    assert_eq!(s.take_work_start_claims("pk", 12).unwrap().len(), 1);
    assert!(s.take_work_start_claims("pk", 13).unwrap().is_empty());
}

#[test]
fn injected_delivery_stages_work_start_for_a_later_hook() {
    let s = Store::open_memory().unwrap();
    upsert_runtime(&s, "pk", 1);
    insert_chat(&s, "evt", 10);
    s.enqueue_inbox("evt", "pk", "human", "room", "start", 10)
        .unwrap();
    s.claim_pending_event_ids_for_pubkey(&["evt".into()], "pk", 11)
        .unwrap();
    s.mark_injected_for_echo(&["evt".into()], "pk", 12).unwrap();

    let handoffs = s.take_work_start_claims("pk", 13).unwrap();
    assert_eq!(handoffs.len(), 1);
    assert_eq!(handoffs[0].event_id, "evt");
}

#[test]
fn pty_submission_requires_prompt_corroboration_before_injected() {
    let s = Store::open_memory().unwrap();
    upsert_runtime(&s, "pk", 1);
    insert_chat(&s, "abcdef1234567890", 10);
    s.enqueue_inbox(
        "abcdef1234567890",
        "pk",
        "human",
        "room",
        "please review",
        10,
    )
    .unwrap();
    s.claim_pending_event_ids_for_pubkey(&["abcdef1234567890".into()], "pk", 11)
        .unwrap();
    s.mark_submitted_for_prompt_confirm(&["abcdef1234567890".into()], "pk", 12)
        .unwrap();
    assert_eq!(state_for(&s, "abcdef1234567890", "pk"), "submitted");
    assert!(s.take_work_start_claims("pk", 13).unwrap().is_empty());

    let confirmed = s
        .confirm_submitted_from_prompt(
            "pk",
            r#"<user_query><mosaico><message id="abcdef" from="@human">please review</message></mosaico></user_query>"#,
            14,
        )
        .unwrap();
    assert_eq!(confirmed, vec!["abcdef1234567890".to_string()]);
    assert_eq!(state_for(&s, "abcdef1234567890", "pk"), "injected");
    assert_eq!(s.take_work_start_claims("pk", 15).unwrap().len(), 1);
}

#[test]
fn unconfirmed_pty_submission_requeues_for_hook_delivery() {
    let s = Store::open_memory().unwrap();
    upsert_runtime(&s, "pk", 1);
    insert_chat(&s, "evt-unconfirmed", 10);
    s.enqueue_inbox("evt-unconfirmed", "pk", "human", "room", "hello", 10)
        .unwrap();
    s.claim_pending_event_ids_for_pubkey(&["evt-unconfirmed".into()], "pk", 11)
        .unwrap();
    s.mark_submitted_for_prompt_confirm(&["evt-unconfirmed".into()], "pk", 12)
        .unwrap();

    // Human turn with unrelated prompt: confirm misses, reenqueue restores pending.
    assert!(s
        .confirm_submitted_from_prompt("pk", "what agents do you see?", 13)
        .unwrap()
        .is_empty());
    assert_eq!(
        s.reenqueue_submitted("pk").unwrap(),
        vec!["evt-unconfirmed".to_string()]
    );
    assert_eq!(state_for(&s, "evt-unconfirmed", "pk"), "pending");
    assert_eq!(s.claim_pending_for_pubkey("pk", 14).unwrap().len(), 1);
}

#[test]
fn direct_message_survives_route_removal_and_rejoin() {
    let s = Store::open_memory().unwrap();
    upsert_runtime(&s, "pk", 1);
    insert_chat(&s, "old-membership", 10);
    s.enqueue_inbox("old-membership", "pk", "human", "room", "queued", 10)
        .unwrap();

    s.revoke_route_and_mark_absent("pk", "room", 11).unwrap();
    s.grant_session_route("pk", "room", 12).unwrap();

    let claimed = s.claim_pending_for_pubkey("pk", 13).unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].event_id, "old-membership");
    assert_eq!(state_for(&s, "old-membership", "pk"), "delivered");
}

#[test]
fn direct_message_parked_before_registration_is_claimable_after_registration() {
    let s = Store::open_memory().unwrap();
    assert!(s
        .enqueue_inbox("pre-route", "pk", "human", "room", "queued", 10)
        .unwrap());
    upsert_runtime(&s, "pk", 11);

    let claimed = s.claim_pending_for_pubkey("pk", 12).unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].event_id, "pre-route");
}

#[test]
fn same_event_is_independent_per_pubkey() {
    let s = Store::open_memory().unwrap();
    assert!(s
        .enqueue_inbox("evt", "pk-a", "sender", "room", "hello", 10)
        .unwrap());
    assert!(s
        .enqueue_inbox("evt", "pk-b", "sender", "room", "hello", 10)
        .unwrap());
    assert!(!s
        .enqueue_inbox("evt", "pk-a", "sender", "room", "hello", 10)
        .unwrap());
}

fn upsert_runtime(store: &Store, pubkey: &str, now: u64) {
    store
        .reserve_hook_session_for_test(&crate::state::RegisterSession {
            pubkey: pubkey.into(),
            observed_harness: "codex".into(),
            agent_slug: "codex".into(),
            launch_channel_h: "room".into(),
            work_root: "room".into(),
            child_pid: None,
            now,
        })
        .unwrap();
}

fn insert_chat(store: &Store, event_id: &str, created_at: u64) {
    store
        .insert_event(&RelayEvent {
            id: event_id.into(),
            kind: 9,
            pubkey: "human".into(),
            created_at,
            channel_h: "room".into(),
            d_tag: String::new(),
            content: event_id.into(),
            tags_json: "[]".into(),
        })
        .unwrap();
}
