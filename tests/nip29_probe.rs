//! PROBE (ignored by default; run explicitly against the live NIP-29 relay):
//!
//!   TE_NIP29_RELAY=<relay> cargo test --test nip29_probe -- --ignored --nocapture
//!
//! Gates the "daemon owns NIP-29 groups" feature. The relay's exact rules are the
//! only real unknown and the codebase can't answer them — this asks the relay
//! directly (default wss://nip29.f7z.io, or $TE_NIP29_RELAY). It walks the full
//! create → add-member → write lifecycle and reports, in order:
//!
//! Publishes disposable `te-probe-*` groups and kind:1 events, can be
//! rate-limited, and is not part of default CI or routine local regression tests.
//!
//!   1. Does 9007 honor a CLIENT-CHOSEN group id via the `h` tag, or does the
//!      relay assign its own id? (Load-bearing: model assumes group id == slug.)
//!   2. Can you 9007 a slug that already has passive activity? ("already exists"?)
//!   3. Ordering: does put-user (9000) need the group to exist first? What role
//!      tag shape does the relay want?
//!   4. Are `previous`/timeline-reference tags required on 9007/9000?
//!   5. Default access mode: is a fresh group already closed, or must we 9002 it
//!      closed? Confirmed by posting a non-member event after create+add.
//!   6. After a `closed`+`public` 9002 lock, can a NON-member daemon connection
//!      still READ the group?
//!
//! VALIDATED FINDINGS (nip29.f7z.io, 2026-06-09): id honored ✓; no `previous` tags
//! needed ✓; a fresh group is OPEN by default, so membership is enforced only
//! after 9002 `["closed"]`; use `["public"]` so the non-member daemon connection
//! can still read. Recipe: 9007 create → 9002 closed+public → 9000 put-user.
//!
//! Hard asserts: id honored, lock enforces membership (non-member write blocked),
//! and non-member read stays open. Other answers are reported via eprintln.

#[path = "nip29_probe/support.rs"]
mod support;

use nostr_sdk::prelude::*;
use std::time::Duration;
use support::*;

#[tokio::test]
#[ignore]
async fn nip29_group_lifecycle() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let relay = relay_url();
    eprintln!("\n[probe] ===== NIP-29 group lifecycle probe =====");
    eprintln!("[probe] relay = {relay}");

    let admin = Keys::generate(); // operator / userNsec
    let member = Keys::generate(); // an agent we add
    let outsider = Keys::generate(); // an agent we never add
    let slug = unique_slug();
    eprintln!("[probe] slug (client-chosen id) = {slug}");
    eprintln!("[probe] admin   = {}", admin.public_key().to_hex());
    eprintln!("[probe] member  = {}", member.public_key().to_hex());
    eprintln!("[probe] outsider= {}", outsider.public_key().to_hex());

    let admin_c = connect(admin.clone(), &relay).await;

    // ── Q1/Q4: create-group (9007) with a client-chosen h-tag id, no `previous`.
    // Retry on rate-limiting (relay29 throttles repeated runs) with backoff; if it
    // never clears, SKIP rather than fail — rate-limiting is environmental.
    let created_ok = create_group_with_retry(&admin, &admin_c, &slug).await;
    if !created_ok {
        eprintln!(
            "[probe] SKIP: 9007 create-group stayed rate-limited — rerun later. \
             (Recipe already validated; this is environmental.)"
        );
        admin_c.disconnect().await;
        return;
    }

    tokio::time::sleep(Duration::from_millis(800)).await;

    // Q1: did a group with OUR id materialize? Read relay-authored 39000/39001/39002 by #d=slug.
    let id_honored = group_id_honored(&admin_c, &slug, created_ok).await;

    // ── Q3: put-user (9000) adding `member`. Try role tag ["p", pk, "member"].
    let put = EventBuilder::new(Kind::from(KIND_PUT_USER), "")
        .tags([
            h_tag(&slug),
            Tag::custom(
                TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::P)),
                [member.public_key().to_hex(), "member".to_string()],
            ),
        ])
        .build(admin.public_key());
    let put = admin.sign_event(put).await.expect("sign put-user");
    let put_ok = publish(&admin_c, &put, "9000 put-user (role=member)").await;

    tokio::time::sleep(Duration::from_millis(800)).await;
    let members = fetch(
        &admin_c,
        Filter::new()
            .kind(Kind::from(KIND_GROUP_MEMBERS))
            .identifier(&slug),
        "39002 members (#d=slug)",
    )
    .await;
    let member_listed = members.iter().any(|e| {
        e.tags
            .iter()
            .any(|t| t.as_slice().get(1).map(|s| s.as_str()) == Some(&member.public_key().to_hex()))
    });
    eprintln!(
        "[probe] Q3 put-user accepted={put_ok}, member appears in 39002={}",
        member_listed
    );

    // ── Q5: access enforcement. member (added) vs outsider (not added) each post
    // a kind:1 with the group h-tag; read back to see which the relay accepted.
    let member_c = connect(member.clone(), &relay).await;
    let outsider_c = connect(outsider.clone(), &relay).await;

    let m_marker = format!("te-probe-member-{}", slug);
    let m_note = EventBuilder::new(Kind::from(KIND_NOTE), &m_marker)
        .tags([h_tag(&slug)])
        .build(member.public_key());
    let m_note = member.sign_event(m_note).await.unwrap();
    publish(&member_c, &m_note, "member kind:1 into group").await;

    let o_marker = format!("te-probe-outsider-{}", slug);
    let o_note = EventBuilder::new(Kind::from(KIND_NOTE), &o_marker)
        .tags([h_tag(&slug)])
        .build(outsider.public_key());
    let o_note = outsider.sign_event(o_note).await.unwrap();
    publish(&outsider_c, &o_note, "outsider kind:1 into group").await;

    tokio::time::sleep(Duration::from_millis(800)).await;
    let notes = fetch(
        &admin_c,
        Filter::new()
            .kind(Kind::from(KIND_NOTE))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::H), &slug),
        "kind:1 in group (readback)",
    )
    .await;
    let member_accepted = notes.iter().any(|e| e.content == m_marker);
    let outsider_accepted = notes.iter().any(|e| e.content == o_marker);
    eprintln!(
        "[probe] Q5 enforcement: member_note_accepted={member_accepted} outsider_note_accepted={outsider_accepted}"
    );
    eprintln!(
        "[probe] Q5 => fresh group is {}",
        if member_accepted && !outsider_accepted {
            "CLOSED by default (non-member rejected) — no 9002 lock needed"
        } else if outsider_accepted {
            "OPEN by default — must 9002 it closed to enforce membership"
        } else {
            "UNKNOWN (member rejected too — check ordering / previous tags / role)"
        }
    );

    // ── Q5b: lock the group with 9002 edit-metadata (closed+public), then retest.
    // closed => only members may write; public => anyone (incl. the non-member
    // daemon AUTH key) may still read.
    eprintln!("\n[probe] ----- Q5b: 9002 edit-metadata lock (closed+public) -----");
    let edit = EventBuilder::new(Kind::from(9002u16), "")
        .tags([
            h_tag(&slug),
            Tag::custom(TagKind::Custom("name".into()), [slug.clone()]),
            Tag::custom(TagKind::Custom("closed".into()), Vec::<String>::new()),
            Tag::custom(TagKind::Custom("public".into()), Vec::<String>::new()),
        ])
        .build(admin.public_key());
    let edit = admin.sign_event(edit).await.expect("sign 9002");
    publish(&admin_c, &edit, "9002 edit-metadata closed+public").await;
    tokio::time::sleep(Duration::from_millis(900)).await;

    let m2 = format!("te-probe-member2-{slug}");
    let m2e = EventBuilder::new(Kind::from(KIND_NOTE), &m2)
        .tags([h_tag(&slug)])
        .build(member.public_key());
    let m2e = member.sign_event(m2e).await.unwrap();
    let member_write_after_lock = publish(&member_c, &m2e, "member kind:1 AFTER lock").await;

    let o2 = format!("te-probe-outsider2-{slug}");
    let o2e = EventBuilder::new(Kind::from(KIND_NOTE), &o2)
        .tags([h_tag(&slug)])
        .build(outsider.public_key());
    let o2e = outsider.sign_event(o2e).await.unwrap();
    let outsider_write_after_lock = publish(&outsider_c, &o2e, "outsider kind:1 AFTER lock").await;
    eprintln!(
        "[probe] Q5b AFTER LOCK: member_write={member_write_after_lock} outsider_write={outsider_write_after_lock} (outsider expected REJECTED)"
    );

    // ── Q6: a NON-member connection (stand-in for the daemon AUTH key) reads the
    // closed+public group.
    eprintln!("\n[probe] ----- Q6: non-member read of closed+public group -----");
    let reader_c = connect(Keys::generate(), &relay).await;
    let rnotes = fetch(
        &reader_c,
        Filter::new()
            .kind(Kind::from(KIND_NOTE))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::H), &slug),
        "non-member reads group kind:1",
    )
    .await;
    let non_member_can_read = rnotes
        .iter()
        .any(|e| e.content.starts_with("te-probe-member"));
    eprintln!(
        "[probe] Q6 => non-member (daemon-key) CAN read closed+public group: {non_member_can_read}"
    );

    // ── Q7: PRODUCTION TOPOLOGY. The daemon has ONE connection authed as a
    // non-member key and signs each event with a *different* key (agent or admin).
    // Load-bearing: does relay29 authorize writes by the event's AUTHOR or by the
    // connection's AUTH identity? If by AUTH identity, every agent presence write
    // and every group-management event over the daemon's connection is blocked and
    // the whole feature is broken. `reader_c` is authed as a non-member (the daemon
    // stand-in); we publish a member-signed note and an admin-signed 9000 over it.
    eprintln!("\n[probe] ----- Q7: writes signed by X over a NON-member connection -----");
    let m3 = format!("te-probe-member3-{slug}");
    let m3e = EventBuilder::new(Kind::from(KIND_NOTE), &m3)
        .tags([h_tag(&slug)])
        .build(member.public_key());
    let m3e = member.sign_event(m3e).await.unwrap();
    publish(&reader_c, &m3e, "member-signed note over NON-member conn").await;

    let newmember = Keys::generate();
    let put2 = EventBuilder::new(Kind::from(KIND_PUT_USER), "")
        .tags([
            h_tag(&slug),
            Tag::custom(
                TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::P)),
                [newmember.public_key().to_hex(), "member".to_string()],
            ),
        ])
        .build(admin.public_key());
    let put2 = admin.sign_event(put2).await.unwrap();
    publish(&reader_c, &put2, "admin-signed 9000 over NON-member conn").await;
    tokio::time::sleep(Duration::from_millis(900)).await;

    let notes3 = fetch(
        &reader_c,
        Filter::new()
            .kind(Kind::from(KIND_NOTE))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::H), &slug),
        "kind:1 in group (topology readback)",
    )
    .await;
    let topo_member_write = notes3.iter().any(|e| e.content == m3);
    let members3 = fetch(
        &reader_c,
        Filter::new()
            .kind(Kind::from(KIND_GROUP_MEMBERS))
            .identifier(&slug),
        "39002 members (topology readback)",
    )
    .await;
    let topo_admin_write = members3.iter().any(|e| {
        e.tags.iter().any(|t| {
            t.as_slice().get(1).map(|s| s.as_str()) == Some(&newmember.public_key().to_hex())
        })
    });
    eprintln!(
        "[probe] Q7 => member-signed write over non-member conn accepted: {topo_member_write}"
    );
    eprintln!("[probe] Q7 => admin-signed 9000 over non-member conn accepted: {topo_admin_write}");

    admin_c.disconnect().await;
    member_c.disconnect().await;
    outsider_c.disconnect().await;
    reader_c.disconnect().await;

    eprintln!("\n[probe] ===== SUMMARY =====");
    eprintln!("[probe] Q1 id honored:                 {id_honored}");
    eprintln!("[probe] Q3 member added:               {member_listed}");
    eprintln!("[probe] Q5 fresh group open (pre-lock): {outsider_accepted}");
    eprintln!("[probe] Q5b member writes after lock:  {member_write_after_lock}");
    eprintln!(
        "[probe] Q5b outsider blocked after lock:{}",
        !outsider_write_after_lock
    );
    eprintln!("[probe] Q6 non-member can read:        {non_member_can_read}");
    eprintln!("[probe] Q7 member write over non-member conn: {topo_member_write}");
    eprintln!("[probe] Q7 admin 9000 over non-member conn:   {topo_admin_write}");
    eprintln!("[probe] Recipe: 9007 create -> 9002 closed+public -> 9000 put-user/agent.\n");

    // Load-bearing gates for the daemon-owned-groups design.
    assert!(
        id_honored,
        "relay must honor a client-chosen group id (h-tag on 9007); else group id != slug."
    );
    assert!(
        member_write_after_lock && !outsider_write_after_lock,
        "9002 closed lock must enforce membership: members write, non-members blocked."
    );
    assert!(
        non_member_can_read,
        "closed+public must keep reads open so the non-member daemon connection sees group events."
    );
    assert!(
        topo_member_write && topo_admin_write,
        "PRODUCTION TOPOLOGY: relay29 must authorize writes by the event AUTHOR, not the \
         connection's AUTH identity. The daemon signs each event with the agent/admin key but \
         sends over ONE connection authed as the non-member daemon key; if this fails, agent \
         presence and group management into managed groups are blocked and the design must change."
    );
}
