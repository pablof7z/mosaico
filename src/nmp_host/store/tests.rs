//! The two refusals Mosaico must never confuse, against real stores.
//!
//! Both fixtures are files NMP actually refuses, not hand-built error values.
//! A hand-built `EngineError` proves the `match` arms and nothing about which
//! arm a real store lands in — and "compiles at every arm, wrong at run time"
//! is the exact failure a widened enum ships.

use super::*;

/// A store whose marker this build cannot read. This is the incident's shape:
/// the 1.05 GB store carried a marker at an address a superseded epoch owned,
/// so the current probe found nothing and said so — literally true, and
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

#[test]
fn a_superseded_epoch_is_the_one_condition_that_names_a_discard() {
    let fixture = tempfile::tempdir().expect("temporary directory");
    let path = fixture.path().join("superseded.redb");
    superseded_epoch_store(&path);

    let condition = probe(&path).expect("a superseded-epoch store must be a reported condition");
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
        remedy.contains("mosaico daemon discard-superseded-store"),
        "the one condition a discard fixes must name the discard: {remedy}"
    );
    assert!(
        remedy.contains("lost with the file"),
        "the remedy must state what the permanent discard costs: {remedy}"
    );
}

#[test]
fn damaged_bytes_are_a_condition_that_forbids_the_discard() {
    let fixture = tempfile::tempdir().expect("temporary directory");
    let path = fixture.path().join("damaged.redb");
    std::fs::write(&path, b"not a redb database").expect("damaged fixture must write");

    let condition = probe(&path).expect("damaged bytes must be a reported condition");
    assert!(
        matches!(condition, StoreCondition::Unusable { .. }),
        "damaged bytes must never be reported as a discardable epoch: {condition:?}"
    );
    let remedy = condition.remedy();
    assert!(
        remedy.contains("do NOT delete the store"),
        "the non-epoch remedy must forbid the discard: {remedy}"
    );
    assert!(
        !remedy.contains("discard-superseded-store"),
        "no refusal but the epoch one may point at the discard command: {remedy}"
    );
}

#[test]
fn the_discard_is_refused_on_anything_but_a_superseded_epoch() {
    let fixture = tempfile::tempdir().expect("temporary directory");

    let damaged = fixture.path().join("damaged.redb");
    std::fs::write(&damaged, b"not a redb database").expect("damaged fixture must write");
    let refusal = discard_superseded(&damaged)
        .expect_err("a store that is not a superseded epoch must never be deleted");
    assert!(
        format!("{refusal:#}").contains("refusing to delete"),
        "the refusal must say it refused: {refusal:#}"
    );
    assert!(
        damaged.exists(),
        "the refused discard must leave the operator's bytes on disk"
    );

    let healthy = fixture.path().join("healthy.redb");
    Engine::new(probe_config(&healthy))
        .expect("a fresh store must open")
        .shutdown();
    discard_superseded(&healthy).expect_err("a store NMP opens must never be deleted");
    assert!(healthy.exists(), "a healthy store must survive the attempt");

    let absent = fixture.path().join("absent.redb");
    discard_superseded(&absent).expect_err("there is nothing to discard");
}

#[test]
fn the_discard_a_superseded_epoch_names_is_performable_and_leaves_a_usable_store() {
    let fixture = tempfile::tempdir().expect("temporary directory");
    let path = fixture.path().join("superseded.redb");
    superseded_epoch_store(&path);

    let discarded = discard_superseded(&path).expect("a superseded epoch must be discardable");
    assert!(
        matches!(discarded, StoreCondition::SupersededEpoch { .. }),
        "the discard must report the condition it acted on: {discarded:?}"
    );
    assert!(!path.exists(), "the discard must remove the refused store");
    assert!(
        probe(&path).is_none(),
        "the path an operator was told to clear must open afterwards"
    );
}

/// The daemon's own door, not just the probe: a refusal `NmpHost::open` returns
/// must still be classifiable after `anyhow` has carried it.
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
