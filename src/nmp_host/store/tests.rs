//! The two refusals Mosaico must never confuse, against real stores.
//!
//! Both fixtures are files NMP actually refuses, not hand-built error values.
//! A hand-built `EngineError` proves the `match` arms and nothing about which
//! arm a real store lands in — and "compiles at every arm, wrong at run time"
//! is the exact failure a widened enum ships.

use super::*;

/// A store whose marker this build cannot read. This is the incident's shape:
/// the 1.05 GB store carried a marker at an address a superseded epoch owned,
/// so the current engine found nothing and said so — literally true, and
/// indistinguishable from an unreadable file to anyone holding only the string.
fn superseded_epoch_store(path: &Path) {
    use redb::{Database, TableDefinition};

    let database = Database::create(path).expect("epoch fixture must create");
    let write = database.begin_write().expect("epoch fixture must begin");
    {
        let mut marker = write
            .open_table(TableDefinition::<&str, u64>::new("schema_meta_v6"))
            .expect("epoch fixture must open a retired marker table");
        marker.insert("version", 10u64).expect("marker must insert");
    }
    write.commit().expect("epoch fixture must commit");
}

fn condition_for(path: &Path) -> StoreCondition {
    let error = super::super::NmpHost::open(&[], None, Some(path), &nostr::Keys::generate())
        .err()
        .expect("fixture must make NMP refuse the store");
    StoreCondition::of_open_error(&error).expect("refusal must remain a typed store condition")
}

#[test]
fn a_superseded_epoch_is_the_one_condition_that_names_a_full_reset() {
    let fixture = tempfile::tempdir().expect("temporary directory");
    let path = fixture.path().join("superseded.redb");
    superseded_epoch_store(&path);

    let condition = condition_for(&path);
    let StoreCondition::SupersededEpoch {
        path: named,
        expected,
        found,
    } = &condition
    else {
        panic!("a superseded epoch must not be reported as anything else: {condition:?}");
    };
    assert!(
        named.ends_with("superseded.redb"),
        "the condition must name the store an operator would delete: {named}"
    );
    assert!(*expected > 0, "this build's own epoch must be reported");
    assert_eq!(
        *found, None,
        "a marker this build cannot read is absent, not a different number"
    );
    assert_eq!(condition.state(), "superseded-epoch");

    // `found: None` means "not this epoch" and NEVER "no data". Rendering it as
    // an empty store would cost someone their publish queue on a sentence.
    let summary = condition.summary();
    assert!(
        summary.contains("no schema marker this build can read")
            && summary.contains("this is not a claim the store is empty"),
        "an unreadable marker must not be reported as an empty store: {summary}"
    );
    let remedy = condition.remedy();
    assert!(
        remedy.contains("mosaico daemon reset-state --yes-i-know-this-wipes-local-state"),
        "the one condition a reset fixes must name the full reset: {remedy}"
    );
    assert!(
        remedy.contains("all local Mosaico runtime state"),
        "the remedy must state the full reset scope: {remedy}"
    );
}

#[test]
fn damaged_bytes_are_a_condition_that_offers_no_reset() {
    let fixture = tempfile::tempdir().expect("temporary directory");
    let path = fixture.path().join("damaged.redb");
    std::fs::write(&path, b"not a redb database").expect("damaged fixture must write");

    let condition = condition_for(&path);
    assert!(
        matches!(condition, StoreCondition::Unusable { .. }),
        "damaged bytes must never be reported as a resettable epoch: {condition:?}"
    );
    let remedy = condition.remedy();
    assert!(
        remedy.contains("do NOT delete the store"),
        "the non-epoch remedy must forbid deletion: {remedy}"
    );
    assert!(
        !remedy.contains("reset-state"),
        "no refusal but the epoch one may point at the reset command: {remedy}"
    );
}

#[test]
fn nmp_owned_reset_removes_a_closed_superseded_store_and_is_idempotent() {
    let fixture = tempfile::tempdir().expect("temporary directory");
    let path = fixture.path().join("superseded.redb");
    superseded_epoch_store(&path);

    reset(&path).expect("a closed superseded store must be resettable");
    assert!(!path.exists(), "NMP's reset must remove the complete store");
    let reopened = super::super::NmpHost::open(&[], None, Some(&path), &nostr::Keys::generate())
        .expect("the path an operator was told to clear must open afterwards");
    drop(reopened);
    reset(&path).expect("a missing store is already reset");
}

/// A refusal `NmpHost::open` returns must still be classifiable after `anyhow`
/// has carried it.
#[test]
fn the_condition_survives_the_host_open_door_as_a_type_not_a_message() {
    let fixture = tempfile::tempdir().expect("temporary directory");
    let path = fixture.path().join("superseded.redb");
    superseded_epoch_store(&path);

    let error = super::super::NmpHost::open(&[], None, Some(&path), &nostr::Keys::generate())
        .err()
        .expect("a superseded-epoch store must refuse the host open");
    let condition = StoreCondition::of_open_error(&error)
        .expect("the typed refusal must survive the host's error type");
    assert!(
        matches!(condition, StoreCondition::SupersededEpoch { .. }),
        "the host door must not flatten the epoch refusal: {condition:?}"
    );

    // Not every open failure is the store's fault, and inventing a store
    // condition for one would send an operator to delete a healthy file.
    let unrelated = super::super::NmpHost::open(
        &["not a relay url".to_string()],
        None,
        None,
        &nostr::Keys::generate(),
    )
    .err()
    .expect("an unparseable relay must refuse the host open");
    assert!(
        StoreCondition::of_open_error(&unrelated).is_none(),
        "a failure that is not about the store must not be reported as one"
    );
}
