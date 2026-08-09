use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::args::{Args, DemandShape, LifecycleSchedule, OutputFormat, Scenario, TopologyChoice};

const DISPLAY_ROOT: &str = "target/nmp-stress-failures";

pub(crate) struct RunIdentity<'a> {
    pub(crate) nmp_revision: &'a str,
    pub(crate) harness_revision: &'a str,
    pub(crate) lock_hash: &'a str,
}

pub(crate) fn run_seeded_matrix<T>(
    args: &Args,
    identity: RunIdentity<'_>,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(DISPLAY_ROOT);
    run_seeded_matrix_at(args, identity, &root, DISPLAY_ROOT, operation)
}

fn run_seeded_matrix_at<T>(
    args: &Args,
    identity: RunIdentity<'_>,
    root: &Path,
    display_root: &str,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let failure = match operation() {
        Ok(value) => return Ok(value),
        Err(error) => error,
    };
    let replay = replay_command(args);
    let failure_hash = format!("{:x}", Sha256::digest(format!("{failure:#}").as_bytes()));
    match retain(root, display_root, args, identity, &replay, &failure_hash) {
        Ok(path) => bail!(
            "seeded matrix failed (failure_sha256={failure_hash}); failure artifact retained at {path}; exact replay command: {replay}"
        ),
        Err(_) => bail!(
            "seeded matrix failed (failure_sha256={failure_hash}); failure artifact retention failed; exact replay command: {replay}"
        ),
    }
}

fn retain(
    root: &Path,
    display_root: &str,
    args: &Args,
    identity: RunIdentity<'_>,
    replay: &str,
    failure_hash: &str,
) -> Result<String> {
    fs::create_dir_all(root).context("creating failure artifact directory")?;
    let recorded_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock precedes Unix epoch")?
        .as_millis();
    let stem = format!("matrix-seed-{}-{recorded_unix_ms}", args.seed);
    let contents = format!(
        "format_version=1\nstatus=failure\nscenario=matrix\nseed={}\nrecorded_unix_ms={recorded_unix_ms}\nnmp_revision={}\nharness_revision={}\nlock_sha256={}\nfailure_sha256={failure_hash}\nreplay_command={replay}\n",
        args.seed, identity.nmp_revision, identity.harness_revision, identity.lock_hash,
    );
    for attempt in 0..1_000u16 {
        let suffix = if attempt == 0 {
            String::new()
        } else {
            format!("-{attempt}")
        };
        let filename = format!("{stem}{suffix}.txt");
        let path = root.join(&filename);
        match write_new(&path, contents.as_bytes()) {
            Ok(()) => return Ok(format!("{display_root}/{filename}")),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error).context("writing failure artifact"),
        }
    }
    bail!("failure artifact name space exhausted for this millisecond")
}

fn write_new(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(contents)?;
    file.sync_all()
}

fn replay_command(args: &Args) -> String {
    format!(
        "env -u MOSAICO cargo run --release --locked --features stress-harness --bin nmp-subscription-stress -- --scenario {} --topology {} --format {} --retained {} --mailboxes {} --profile-burst {} --burst-size {} --shard-size {} --corpus-rows {} --iterations {} --hold-ms {} --seed {} --demand-shape {} --lifecycle-schedule {} --matrix-counts {}",
        scenario(args.scenario),
        topology(args.topology),
        output(args.format),
        args.retained,
        args.mailboxes,
        args.profile_burst,
        args.burst_size,
        args.shard_size,
        args.corpus_rows,
        args.iterations,
        args.hold_ms,
        args.seed,
        demand_shape(args.demand_shape),
        lifecycle_schedule(args.lifecycle_schedule),
        args.matrix_counts
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(","),
    )
}

const fn scenario(value: Scenario) -> &'static str {
    match value {
        Scenario::Captured => "captured",
        Scenario::Facade => "facade",
        Scenario::Store => "store",
        Scenario::Router => "router",
        Scenario::Consumer => "consumer",
        Scenario::Profiles => "profiles",
        Scenario::Freshness => "freshness",
        Scenario::Matrix => "matrix",
    }
}

const fn topology(value: TopologyChoice) -> &'static str {
    match value {
        TopologyChoice::PerIdentity => "per-identity",
        TopologyChoice::Sharded => "sharded",
        TopologyChoice::Both => "both",
    }
}

const fn output(value: OutputFormat) -> &'static str {
    match value {
        OutputFormat::Human => "human",
        OutputFormat::Csv => "csv",
    }
}

const fn demand_shape(value: DemandShape) -> &'static str {
    match value {
        DemandShape::All => "all",
        DemandShape::ExactDuplicate => "exact-duplicate",
        DemandShape::CompatibleDistinct => "compatible-distinct",
        DemandShape::ProfileAuthors => "profile-authors",
        DemandShape::LimitedIncompatible => "limited-incompatible",
        DemandShape::UnlimitedMultiAxisIncompatible => "unlimited-multi-axis-incompatible",
    }
}

const fn lifecycle_schedule(value: LifecycleSchedule) -> &'static str {
    match value {
        LifecycleSchedule::All => "all",
        LifecycleSchedule::Forward => "forward",
        LifecycleSchedule::Reverse => "reverse",
        LifecycleSchedule::SeededRandom => "seeded-random",
        LifecycleSchedule::BeforeAdmission => "before-admission",
        LifecycleSchedule::Interleaved => "interleaved",
    }
}

#[cfg(test)]
#[path = "failure_artifact/tests.rs"]
mod tests;
