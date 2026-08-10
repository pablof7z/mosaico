use anyhow::anyhow;
use clap::Parser;

use super::*;

const REVISION: &str = "0123456789abcdef0123456789abcdef01234567";
const LOCK_HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn failure_retains_sanitized_artifact_and_exact_replay_command() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("artifacts");
    let args = matrix_args();
    let replay = "env -u MOSAICO cargo run --release --locked --features stress-harness --bin nmp-subscription-stress -- --scenario matrix --topology per-identity --format csv --retained 12 --mailboxes 10 --profile-burst 8 --burst-size 4 --shard-size 3 --corpus-rows 99 --iterations 2 --hold-ms 7 --seed 47 --demand-shape compatible-distinct --lifecycle-schedule seeded-random --matrix-counts 1,32,207";
    assert_eq!(replay_command(&args), replay);
    let result = run_seeded_matrix_at(
        &args,
        identity(),
        &root,
        "target/nmp-stress-failures",
        || Err::<(), _>(anyhow!("secret-value at /Users/alice/private.redb")),
    );
    let message = result.unwrap_err().to_string();
    assert!(message.contains("exact replay command: "));
    assert!(message.contains("--seed 47"));
    assert!(message.contains("--matrix-counts 1,32,207"));
    assert!(!message.contains("secret-value") && !message.contains("/Users/"));

    let paths = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(paths.len(), 1);
    let artifact = fs::read_to_string(&paths[0]).unwrap();
    assert!(artifact.contains("status=failure"));
    assert!(artifact.contains("failure_sha256="));
    assert!(artifact.contains(&format!("replay_command={replay}\n")));
    assert!(!artifact.contains("secret-value") && !artifact.contains("/Users/"));
}

#[test]
fn success_creates_no_artifact_directory() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("artifacts");
    let result = run_seeded_matrix_at(
        &matrix_args(),
        identity(),
        &root,
        "target/nmp-stress-failures",
        || Ok(29),
    );
    assert_eq!(result.unwrap(), 29);
    assert!(!root.exists());
}

fn identity() -> RunIdentity<'static> {
    RunIdentity {
        nmp_revision: REVISION,
        harness_revision: REVISION,
        lock_hash: LOCK_HASH,
    }
}

fn matrix_args() -> Args {
    Args::parse_from([
        "stress",
        "--scenario",
        "matrix",
        "--topology",
        "per-identity",
        "--format",
        "csv",
        "--retained",
        "12",
        "--mailboxes",
        "10",
        "--profile-burst",
        "8",
        "--burst-size",
        "4",
        "--shard-size",
        "3",
        "--corpus-rows",
        "99",
        "--iterations",
        "2",
        "--hold-ms",
        "7",
        "--seed",
        "47",
        "--demand-shape",
        "compatible-distinct",
        "--lifecycle-schedule",
        "seeded-random",
        "--matrix-counts",
        "1,32,207",
    ])
}
