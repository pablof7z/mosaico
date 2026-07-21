use super::*;

#[test]
fn record_round_trips() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("process.json");
    let expected = RelayProcess {
        pid: 4242,
        endpoint: "ws://127.0.0.1:9888".into(),
    };

    write_record(&path, &expected).unwrap();
    let actual = read_record(&path).unwrap().unwrap();

    assert_eq!(actual.pid, expected.pid);
    assert_eq!(actual.endpoint, expected.endpoint);
}

#[test]
fn unsafe_pids_are_rejected() {
    for value in [0, 1] {
        assert!(pid(value).unwrap_err().to_string().contains("unsafe"));
    }
}
