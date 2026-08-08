use std::time::Instant;

use anyhow::{Context, Result};
use nmp_resolver::Engine;
use nmp_store::RedbStore;

use crate::args::Topology;
use crate::measure::{Metric, Samples};
use crate::store::DisposableStore;
use crate::workload::Workload;

pub(crate) fn run_resolver(
    workload: &Workload,
    fixture: &DisposableStore,
    topology: Topology,
) -> Result<Vec<Metric>> {
    let store = RedbStore::open(fixture.path()).context("opening resolver store control")?;
    store.reset_query_work();
    let mut resolver = Engine::new(store);
    let demands = workload.retained_demands(topology)?;
    let started = Instant::now();
    let mut open_samples = Samples::default();
    let mut handles = Vec::with_capacity(demands.len());
    for demand in demands {
        let (handle, _) = open_samples
            .record(|| resolver.subscribe(demand))
            .context("opening resolver control")?;
        handles.push(handle);
    }
    let open_elapsed = started.elapsed();
    let (index_rows, event_values, examined_rows) = resolver.store().query_work();
    let graph_nodes = resolver.graph_snapshot().nodes.len();
    let active_atoms = resolver.active_demand().len();
    let metrics = resolver.metrics().clone();
    let open = Metric::new(
        "internal_control",
        "resolver_incremental_open",
        topology.label(),
        open_elapsed,
        open_samples,
    )
    .count("observations", handles.len() as u64)
    .count("graph_nodes", graph_nodes as u64)
    .count("active_atoms", active_atoms as u64)
    .count("index_rows", index_rows)
    .count("event_values", event_values)
    .count("examined_rows", examined_rows)
    .count("sets_reevaluated", metrics.sets_reevaluated)
    .note("direct resolver subscribe over Redb; excludes acquisition-core projection and router");

    let started = Instant::now();
    let mut close_samples = Samples::default();
    for handle in &handles {
        close_samples.record(|| resolver.unsubscribe(handle.id()));
    }
    let close = Metric::new(
        "internal_control",
        "resolver_incremental_close",
        topology.label(),
        started.elapsed(),
        close_samples,
    )
    .count("observations", handles.len() as u64)
    .count("active_atoms_after", resolver.active_demand().len() as u64)
    .note("explicit resolver unsubscribe; excludes public channels and consumer teardown");
    Ok(vec![open, close])
}
