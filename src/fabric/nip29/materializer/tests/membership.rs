use super::*;

#[test]
fn older_member_roster_materialization_does_not_replace_newer_roster() {
    let store = Store::open_memory().unwrap();
    let relay = Keys::generate();
    let old_member = Keys::generate().public_key().to_hex();
    let new_member = Keys::generate().public_key().to_hex();

    let newer = build_at(
        &relay,
        39002,
        "",
        vec![make_tag(&["d", "proj"]), make_tag(&["p", &new_member])],
        20,
    );
    let older = build_at(
        &relay,
        39002,
        "",
        vec![make_tag(&["d", "proj"]), make_tag(&["p", &old_member])],
        10,
    );
    Nip29Materializer::materialize_group_snapshot(&store, &snapshot_of("proj", None, Some(&newer)));
    Nip29Materializer::materialize_group_snapshot(&store, &snapshot_of("proj", None, Some(&older)));

    assert!(store.is_channel_member("proj", &new_member).unwrap());
    assert!(!store.is_channel_member("proj", &old_member).unwrap());
}

#[test]
fn older_admin_roster_materialization_does_not_replace_newer_roster() {
    let store = Store::open_memory().unwrap();
    let relay = Keys::generate();
    let old_admin = Keys::generate().public_key().to_hex();
    let new_admin = Keys::generate().public_key().to_hex();

    let newer = build_at(
        &relay,
        39001,
        "",
        vec![make_tag(&["d", "proj"]), make_tag(&["p", &new_admin])],
        20,
    );
    let older = build_at(
        &relay,
        39001,
        "",
        vec![make_tag(&["d", "proj"]), make_tag(&["p", &old_admin])],
        10,
    );
    Nip29Materializer::materialize_group_snapshot(&store, &snapshot_of("proj", Some(&newer), None));
    Nip29Materializer::materialize_group_snapshot(&store, &snapshot_of("proj", Some(&older), None));

    assert!(store.is_channel_admin("proj", &new_admin).unwrap());
    assert!(!store.is_channel_admin("proj", &old_admin).unwrap());
}

/// THE role default this projection removes.
///
/// NIP-29 spells an admin row `["p", <pubkey>]` or `["p", <pubkey>, <role>]` —
/// the role position is optional and free-form. Membership of kind:39001 IS the
/// admin grant; the label beside it is decoration. Mosaico used to read a
/// role-less row as the literal role `"member"`, which silently demoted an
/// admin the relay had in fact granted, and every `role == "admin"` check
/// downstream then denied that identity its own authority.
#[test]
fn an_admin_row_carrying_no_role_is_still_an_admin() {
    let store = Store::open_memory().unwrap();
    let relay = Keys::generate();
    let unlabelled = Keys::generate().public_key().to_hex();
    let labelled = Keys::generate().public_key().to_hex();
    let odd_label = Keys::generate().public_key().to_hex();

    let admins = build_at(
        &relay,
        39001,
        "",
        vec![
            make_tag(&["d", "proj"]),
            make_tag(&["p", &unlabelled]),
            make_tag(&["p", &labelled, "admin"]),
            make_tag(&["p", &odd_label, "moderator"]),
        ],
        30,
    );
    Nip29Materializer::materialize_group_snapshot(
        &store,
        &snapshot_of("proj", Some(&admins), None),
    );

    assert!(store.is_channel_admin("proj", &unlabelled).unwrap());
    assert!(store.is_channel_admin("proj", &labelled).unwrap());
    // A role the relay invented is still an entry on the ADMIN list.
    assert!(store.is_channel_admin("proj", &odd_label).unwrap());
}

/// A relay that publishes an EMPTY member list has published an empty member
/// list, and the cache must say so. This is kept apart from "no host has
/// published one at all", which leaves the cache untouched — the two are
/// indistinguishable in the folded vector and are told apart by the per-host
/// records.
#[test]
fn an_observed_empty_member_list_clears_the_cached_members() {
    let store = Store::open_memory().unwrap();
    let relay = Keys::generate();
    let member = Keys::generate().public_key().to_hex();

    let listed = build_at(
        &relay,
        39002,
        "",
        vec![make_tag(&["d", "proj"]), make_tag(&["p", &member])],
        10,
    );
    Nip29Materializer::materialize_group_snapshot(
        &store,
        &snapshot_of("proj", None, Some(&listed)),
    );
    assert!(store.is_channel_member("proj", &member).unwrap());

    // No host published anything: nothing is asserted, so nothing changes.
    Nip29Materializer::materialize_group_snapshot(&store, &snapshot_of("proj", None, None));
    assert!(store.is_channel_member("proj", &member).unwrap());

    // A host published an empty list: that IS an assertion, and it applies.
    let emptied = build_at(&relay, 39002, "", vec![make_tag(&["d", "proj"])], 20);
    Nip29Materializer::materialize_group_snapshot(
        &store,
        &snapshot_of("proj", None, Some(&emptied)),
    );
    assert!(!store.is_channel_member("proj", &member).unwrap());
}
