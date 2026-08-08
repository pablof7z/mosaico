use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use nmp::{Engine, EngineConfig, Subscription, Window};

use crate::args::{Args, Topology};
use crate::measure::{elapsed_since, process_cpu_time, Metric, Samples};
use crate::workload::Workload;

pub(crate) fn run_retained(
    args: &Args,
    workload: &Workload,
    topology: Topology,
    store_path: Option<&Path>,
    drain_threads: bool,
) -> Result<Vec<Metric>> {
    let store = store_path.map_or("memory", |_| "redb");
    let mode = if drain_threads {
        "threads"
    } else {
        "undrained"
    };
    let label = format!("{}:{store}:{mode}", topology.label());
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
    let queries = workload.retained_queries(topology)?;
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

pub(crate) fn run_profile_burst(
    args: &Args,
    workload: &Workload,
    store_path: &Path,
) -> Result<Vec<Metric>> {
    if args.profile_burst == 0 {
        return Ok(Vec::new());
    }
    let engine = Engine::new(EngineConfig {
        store_path: Some(store_path.to_string_lossy().into_owned()),
        ..EngineConfig::default()
    })?;
    let bound = NonZeroUsize::new(1).expect("one is non-zero");
    let phase_started = Instant::now();
    let mut open_samples = Samples::default();
    let mut first_frame_samples = Samples::default();
    let mut lifecycle_samples = Samples::default();
    let mut frames = 0usize;
    for start in (0..args.profile_burst).step_by(args.burst_size) {
        let end = (start + args.burst_size).min(args.profile_burst);
        let results = thread::scope(|scope| {
            let mut workers = Vec::with_capacity(end - start);
            for index in start..end {
                let query = workload.profile_query(index);
                let engine = &engine;
                workers.push(
                    scope.spawn(move || -> Result<(Duration, Duration, Duration)> {
                        let query = query?;
                        let lifecycle_started = Instant::now();
                        let open_started = Instant::now();
                        let subscription = engine.observe(
                            query,
                            Some(Window::Expandable {
                                initial: bound,
                                max: bound,
                            }),
                        )?;
                        let open = open_started.elapsed();
                        let frame_started = Instant::now();
                        subscription
                            .recv_timeout(Duration::from_secs(5))
                            .context("waiting for cache-only profile frame")?;
                        let first_frame = frame_started.elapsed();
                        drop(subscription);
                        Ok((open, first_frame, lifecycle_started.elapsed()))
                    }),
                );
            }
            workers
                .into_iter()
                .map(|worker| {
                    worker
                        .join()
                        .map_err(|_| anyhow!("profile worker panicked"))?
                })
                .collect::<Result<Vec<_>>>()
        })?;
        for (open, first_frame, lifecycle) in results {
            open_samples.push(open);
            first_frame_samples.push(first_frame);
            lifecycle_samples.push(lifecycle);
            frames += 1;
        }
    }
    let phase_elapsed = phase_started.elapsed();
    engine.shutdown();
    let common = |metric: Metric| {
        metric
            .count("lookups", args.profile_burst as u64)
            .count(
                "peak_workers",
                args.burst_size.min(args.profile_burst) as u64,
            )
            .count("frames", frames as u64)
    };
    Ok(vec![
        common(
            Metric::new(
                "public_facade",
                "profile_window_open",
                "per_identity:redb",
                phase_elapsed,
                open_samples,
            )
            .note("call latency excludes worker-thread creation; total elapsed includes burst scheduling"),
        ),
        common(Metric::new(
            "public_facade",
            "profile_first_frame",
            "per_identity:redb",
            phase_elapsed,
            first_frame_samples,
        )),
        common(
            Metric::new(
                "public_facade",
                "profile_open_drain_close",
                "per_identity:redb",
                phase_elapsed,
                lifecycle_samples,
            )
            .note("short-lived windowed CacheOnly observation; no relay wait"),
        ),
    ])
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
