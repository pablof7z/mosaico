use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use nmp_store::{EventStore, RedbStore, RelayObserved};
use nostr::{Event, EventBuilder, Kind, Tag, Timestamp};

use crate::args::{Args, Topology};
use crate::measure::{Metric, Samples};
use crate::workload::Workload;

pub(crate) struct DisposableStore {
    _root: tempfile::TempDir,
    path: PathBuf,
}

impl DisposableStore {
    pub(crate) fn seed(args: &Args, workload: &Workload) -> Result<(Self, Metric)> {
        let root = tempfile::Builder::new()
            .prefix("mosaico-nmp-stress-")
            .tempdir()
            .context("creating disposable stress root")?;
        let path = root.path().join("fixture.redb");
        let events = corpus(args, workload)?;
        let relay = RelayObserved::new(workload.relay().clone(), Timestamp::from(1_800_000_000));
        let mut store = RedbStore::open(&path).context("opening disposable redb store")?;
        let started = Instant::now();
        store
            .insert_batch(
                events
                    .into_iter()
                    .map(|event| (event, relay.clone()))
                    .collect(),
            )
            .context("seeding disposable redb store")?;
        let elapsed = started.elapsed();
        drop(store);
        let mut samples = Samples::default();
        samples.push(elapsed);
        let metric = Metric::new("internal_control", "redb_seed", "corpus", elapsed, samples)
            .count("rows", args.corpus_rows as u64)
            .note("fixture construction; excluded from timed read/open phases");
        Ok((Self { _root: root, path }, metric))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

pub(crate) fn run_store_queries(
    args: &Args,
    workload: &Workload,
    fixture: &DisposableStore,
    topology: Topology,
) -> Result<Metric> {
    let store = RedbStore::open(fixture.path()).context("opening store query control")?;
    let filters = workload.store_filters(topology);
    store.reset_query_work();
    let started = Instant::now();
    let mut samples = Samples::default();
    let mut returned = 0usize;
    for _ in 0..args.iterations {
        for filter in &filters {
            let rows = samples
                .record(|| store.query(filter))
                .context("redb query control")?;
            returned += rows.len();
        }
    }
    let elapsed = started.elapsed();
    let (index_rows, event_values, examined_rows) = store.query_work();
    Ok(Metric::new(
        "internal_control",
        "redb_projection_queries",
        topology.label(),
        elapsed,
        samples,
    )
    .count("query_calls", (filters.len() * args.iterations) as u64)
    .count("index_rows", index_rows)
    .count("event_values", event_values)
    .count("examined_rows", examined_rows)
    .count("returned_rows", returned as u64)
    .note("direct nmp-store reads; exact work counters are not exposed through public Engine"))
}

pub(crate) fn run_profile_store_queries(
    args: &Args,
    workload: &Workload,
    fixture: &DisposableStore,
) -> Result<Metric> {
    let store = RedbStore::open(fixture.path()).context("opening profile store control")?;
    store.reset_query_work();
    let started = Instant::now();
    let mut samples = Samples::default();
    let mut returned = 0usize;
    for index in 0..args.profile_burst {
        let filter = workload.profile_store_filter(index);
        let rows = samples
            .record(|| store.query_newest(&filter, 1))
            .context("profile query control")?;
        returned += rows.len();
    }
    let elapsed = started.elapsed();
    let (index_rows, event_values, examined_rows) = store.query_work();
    Ok(Metric::new(
        "internal_control",
        "redb_profile_queries",
        "per_identity",
        elapsed,
        samples,
    )
    .count("query_calls", args.profile_burst as u64)
    .count("index_rows", index_rows)
    .count("event_values", event_values)
    .count("examined_rows", examined_rows)
    .count("returned_rows", returned as u64)
    .note("direct bounded kind:0 author queries; no observation/router lifecycle"))
}

fn corpus(args: &Args, workload: &Workload) -> Result<Vec<Event>> {
    let mut events = Vec::with_capacity(args.corpus_rows);
    for (index, keys) in workload.identities.iter().enumerate() {
        events.push(sign(
            keys,
            Kind::Metadata,
            1_700_000_000 + index as u64,
            Vec::new(),
            format!(r#"{{"name":"stress-{index:04}"}}"#),
        )?);
    }
    let ordinary = args.corpus_rows - events.len();
    for index in 0..ordinary {
        let recipient = &workload.identities[index % workload.identities.len()];
        let author = &workload.identities[(index * 17 + 3) % workload.identities.len()];
        let group = index % args.retained.saturating_sub(args.mailboxes).max(1);
        events.push(sign(
            author,
            Kind::from(9u16),
            1_710_000_000 + index as u64,
            vec![
                Tag::parse(["p", &recipient.public_key().to_hex()])?,
                Tag::parse(["h", &format!("stress-group-{group:04}")])?,
            ],
            format!("stress-event-{index:06}"),
        )?);
    }
    Ok(events)
}

fn sign(
    keys: &nostr::Keys,
    kind: Kind,
    created_at: u64,
    tags: Vec<Tag>,
    content: String,
) -> Result<Event> {
    EventBuilder::new(kind, content)
        .tags(tags)
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .context("signing deterministic fixture event")
}
