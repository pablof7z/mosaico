use super::*;

fn superseded_epoch_store(path: &std::path::Path) {
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

/// The condition an operator reads instead of investigating a database by hand.
#[test]
fn a_superseded_epoch_is_a_named_doctor_condition_with_its_own_fix() {
    let fixture = tempfile::tempdir().expect("temporary directory");
    let path = fixture.path().join("nmp.redb");
    superseded_epoch_store(&path);

    let check = check_for(&path).expect("a refused store must appear in the report");
    assert_eq!(check.name, "nmp.store");
    assert_eq!(check.status, CheckStatus::Error);
    assert_eq!(
        check.state.as_deref(),
        Some("superseded-epoch"),
        "a machine reader must branch without parsing the summary"
    );
    assert_eq!(
        check.path.as_deref().map(|path| path.ends_with("nmp.redb")),
        Some(true),
        "the report must name the store the fix acts on"
    );
    let repair = check.repair.expect("a named condition must state its fix");
    assert!(
        repair.contains("mosaico daemon reset-state --yes-i-know-this-wipes-local-state"),
        "the fix must be the command, not a description of one: {repair}"
    );
}

/// What an operator actually reads. The report is the product here: the whole
/// cost of the incident was a refusal that could not be acted on as written.
#[test]
fn the_rendered_report_names_the_store_the_condition_and_the_command() {
    let fixture = tempfile::tempdir().expect("temporary directory");
    let path = fixture.path().join("nmp.redb");
    superseded_epoch_store(&path);

    let report = super::super::DoctorReport {
        healthy: false,
        fix_attempted: false,
        storage: serde_json::json!({}),
        repairs: Vec::new(),
        checks: vec![
            check_for(&path).expect("a refused store must appear in the report"),
            super::super::Check::new(
                "daemon",
                CheckStatus::Error,
                "cannot connect or start: daemon did not respond",
            )
            .repair(daemon_repair(true)),
        ],
    };
    let rendered = super::super::render::human(&report);

    assert!(rendered.contains("[error] nmp.store:"), "{rendered}");
    assert!(rendered.contains("nmp.redb"), "{rendered}");
    assert!(
        rendered.contains("mosaico daemon reset-state --yes-i-know-this-wipes-local-state"),
        "the operator must be able to act on what they read: {rendered}"
    );
    assert!(
        rendered.contains("act on that first"),
        "the daemon symptom must defer to the named cause: {rendered}"
    );
}

/// The same check must not send an operator to delete a failing disk.
#[test]
fn a_store_fault_that_is_not_the_epoch_names_no_reset() {
    let fixture = tempfile::tempdir().expect("temporary directory");
    let path = fixture.path().join("nmp.redb");
    std::fs::write(&path, b"not a redb database").expect("damaged fixture must write");

    let check = check_for(&path).expect("a refused store must appear in the report");
    assert_eq!(check.state.as_deref(), Some("unusable"));
    let repair = check.repair.expect("a named condition must state its fix");
    assert!(
        repair.contains("do NOT delete the store") && !repair.contains("reset-state"),
        "a fault a reset cannot fix must never point at the reset: {repair}"
    );
}

#[test]
fn a_daemon_failure_the_store_does_not_explain_invents_no_store_fault() {
    let fixture = tempfile::tempdir().expect("temporary directory");

    assert!(
        check_for(&fixture.path().join("absent.redb")).is_none(),
        "a first boot has no store yet, and that is not a fault"
    );
    assert_eq!(
        daemon_repair(false),
        "run `mosaico doctor --fix` for a session-preserving daemon restart",
        "a daemon failure the store does not explain keeps its own fix"
    );
    assert!(
        daemon_repair(true).contains("`nmp.store`"),
        "a daemon failure the store explains must point at the named condition"
    );
    assert!(
        !daemon_repair(true).contains("doctor --fix"),
        "a restart clears none of these, so it must not be offered as the fix"
    );
}
