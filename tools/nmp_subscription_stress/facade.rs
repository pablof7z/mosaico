use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use nmp::{Engine, EngineConfig, Subscription};

use crate::args::{Args, Topology};
use crate::measure::{elapsed_since, process_cpu_time, Metric, Samples};
use crate::workload::Workload;

#[derive(Clone, Copy)]
pub(crate) enum RetainedMode {
    CacheOnly,
    Live,
}

impl RetainedMode {
    const fn label(self) -> &'static str {
        match self {
            Self::CacheOnly => "cache_only",
            Self::Live => "live",
        }
    }
}

pub(crate) fn run_retained(
    args: &Args,
    workload: &Workload,
    topology: Topology,
    store_path: Option<&Path>,
    drain_threads: bool,
    retained_mode: RetainedMode,
) -> Result<Vec<Metric>> {
    let store = store_path.map_or("memory", |_| "redb");
    let mode = if drain_threads {
        "threads"
    } else {
        "undrained"
    };
    let label = format!(
        "{}:{store}:{mode}:{}",
        topology.label(),
        retained_mode.label()
    );
    let config = EngineConfig {
        store_path: store_path.map(|path| path.to_string_lossy().into_owned()),
        ..EngineConfig::default()
    };
    let spawned_before = nmp::nmp_threads_spawned();
    let live_before = nmp::nmp_threads_live();
    let mut engine_samples = Samples::default();
    let engine_started = Instant::now();
    let engine_cpu_started = process_cpu_time();
    let engine = Arc::new(
        engine_samples
            .record(|| Engine::new(config))
            .context("constructing public NMP Engine")?,
    );
    let (construct_elapsed, construct_cpu) = elapsed_since(engine_started, engine_cpu_started);
    let queries = match retained_mode {
        RetainedMode::CacheOnly => workload.retained_queries(topology)?,
        RetainedMode::Live => workload.retained_live_queries(topology)?,
    };
    let frames = Arc::new(AtomicU64::new(0));
    let (ready_tx, ready_rx) = mpsc::sync_channel(queries.len().max(1));
    let mut subscriptions = Vec::new();
    let mut drains = Vec::new();
    let mut cancels = Vec::with_capacity(queries.len());
    let open_started = Instant::now();
    let open_cpu_started = process_cpu_time();
    let mut open_samples = Samples::default();
    for (index, query) in queries.into_iter().enumerate() {
        let subscription = open_samples
            .record(|| engine.observe(query, None))
            .with_context(|| format!("opening retained observation {index}"))?;
        cancels.push(subscription.cancel_handle());
        if drain_threads {
            drains.push(spawn_drain(index, subscription, &ready_tx, &frames)?);
        } else {
            subscriptions.push(subscription);
        }
    }
    drop(ready_tx);
    if drain_threads {
        for index in 0..cancels.len() {
            ready_rx
                .recv_timeout(Duration::from_secs(5))
                .with_context(|| format!("waiting for drain {index} to receive its first frame"))?;
        }
    }
    let (open_elapsed, open_cpu) = elapsed_since(open_started, open_cpu_started);
    let threads_after_open = nmp::nmp_threads_spawned();
    let live_after_open = nmp::nmp_threads_live();

    let idle_started = Instant::now();
    let cpu_started = process_cpu_time();
    thread::sleep(Duration::from_millis(args.hold_ms));
    let (idle_elapsed, idle_cpu) = elapsed_since(idle_started, cpu_started);

    let close_started = Instant::now();
    let close_cpu_started = process_cpu_time();
    let mut close_samples = Samples::default();
    for cancel in &cancels {
        close_samples.record(|| cancel.cancel());
    }
    drop(subscriptions);
    engine.shutdown();
    for drain in drains {
        drain
            .join()
            .map_err(|_| anyhow!("Mosaico-style drain thread panicked"))?;
    }
    let (close_elapsed, close_cpu) = elapsed_since(close_started, close_cpu_started);
    let live_after_close = nmp::nmp_threads_live();
    let boundary = if drain_threads {
        "mosaico_consumer"
    } else {
        "public_facade"
    };
    let common = |metric: Metric| {
        metric
            .count("observations", cancels.len() as u64)
            .count("semantic_values", workload.semantic_values() as u64)
            .count(
                "drain_threads",
                if drain_threads {
                    cancels.len() as u64
                } else {
                    0
                },
            )
            .count(
                "live_observations",
                if matches!(retained_mode, RetainedMode::Live) {
                    cancels.len() as u64
                } else {
                    0
                },
            )
    };
    let construct = common(
        Metric::new(
            "public_facade",
            "engine_construct",
            label.clone(),
            construct_elapsed,
            engine_samples,
        )
        .cpu(construct_cpu)
        .count(
            "nmp_threads_spawned",
            threads_after_open.saturating_sub(spawned_before),
        )
        .count(
            "nmp_threads_live",
            live_after_open.saturating_sub(live_before),
        ),
    );
    let open = common(
        Metric::new(
            boundary,
            "retained_observe_open",
            label.clone(),
            open_elapsed,
            open_samples,
        )
        .cpu(open_cpu)
        .count("frames_drained", frames.load(Ordering::Relaxed))
        .note("Engine::observe total; internal store/router phase counts are not public"),
    );
    let idle = common(
        Metric::new(
            boundary,
            "stable_idle_hold",
            label.clone(),
            idle_elapsed,
            Samples::default(),
        )
        .cpu(idle_cpu)
        .count("frames_drained", frames.load(Ordering::Relaxed))
        .note("standing observations, no query or relay churn"),
    );
    let close = common(
        Metric::new(
            boundary,
            "retained_cancel_close",
            label,
            close_elapsed,
            close_samples,
        )
        .cpu(close_cpu)
        .count("nmp_threads_live_after", live_after_close)
        .note("cancel every observation, shutdown engine, and join every consumer thread"),
    );
    Ok(vec![construct, open, idle, close])
}

fn spawn_drain(
    index: usize,
    subscription: Subscription,
    ready: &mpsc::SyncSender<()>,
    frames: &Arc<AtomicU64>,
) -> Result<thread::JoinHandle<()>> {
    let ready = ready.clone();
    let frames = Arc::clone(frames);
    thread::Builder::new()
        .name(format!("stress-nmp-drain-{index}"))
        .spawn(move || {
            if subscription.recv_timeout(Duration::from_secs(5)).is_ok() {
                frames.fetch_add(1, Ordering::Relaxed);
            }
            let _ = ready.send(());
            while subscription.recv().is_ok() {
                frames.fetch_add(1, Ordering::Relaxed);
            }
        })
        .context("spawning Mosaico-style consumer drain")
}
