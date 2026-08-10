use std::num::NonZeroUsize;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use nmp::{Engine, EngineConfig, Freshness, Window};

use crate::args::Args;
use crate::measure::{elapsed_since, process_cpu_time, Metric, Samples};
use crate::workload::Workload;

pub(crate) fn run_cache_only_burst(
    args: &Args,
    workload: &Workload,
    store_path: &Path,
) -> Result<Vec<Metric>> {
    if args.profile_burst == 0 {
        return Ok(Vec::new());
    }
    let engine = Engine::new(config(store_path))?;
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
                "per_identity:redb:cache_only",
                phase_elapsed,
                open_samples,
            )
            .note("call latency excludes worker-thread creation; total elapsed includes burst scheduling"),
        ),
        common(Metric::new(
            "public_facade",
            "profile_first_frame",
            "per_identity:redb:cache_only",
            phase_elapsed,
            first_frame_samples,
        )),
        common(
            Metric::new(
                "public_facade",
                "profile_open_drain_close",
                "per_identity:redb:cache_only",
                phase_elapsed,
                lifecycle_samples,
            )
            .note("short-lived windowed CacheOnly observation; no relay wait"),
        ),
    ])
}

pub(crate) fn run_live_cohort(
    args: &Args,
    workload: &Workload,
    store_path: &Path,
) -> Result<Vec<Metric>> {
    if args.profile_burst == 0 {
        return Ok(Vec::new());
    }
    let engine = Engine::new(config(store_path))?;
    let phase_started = Instant::now();
    let open_cpu_started = process_cpu_time();
    let mut open_samples = Samples::default();
    let mut subscriptions = Vec::with_capacity(args.profile_burst);
    let mut cancels = Vec::with_capacity(args.profile_burst);
    for index in 0..args.profile_burst {
        let query = workload.profile_query_with_freshness(index, Freshness::Live)?;
        let subscription = open_samples.record(|| engine.observe(query, None))?;
        cancels.push(subscription.cancel_handle());
        subscriptions.push(subscription);
    }
    let (open_elapsed, open_cpu) = elapsed_since(phase_started, open_cpu_started);

    let hold_ms = args.hold_ms.max(25);
    let hold_started = Instant::now();
    let hold_cpu_started = process_cpu_time();
    thread::sleep(Duration::from_millis(hold_ms));
    let (hold_elapsed, hold_cpu) = elapsed_since(hold_started, hold_cpu_started);

    let close_started = Instant::now();
    let close_cpu_started = process_cpu_time();
    let mut close_samples = Samples::default();
    for cancel in &cancels {
        close_samples.record(|| cancel.cancel());
    }
    drop(subscriptions);
    engine.shutdown();
    let (close_elapsed, close_cpu) = elapsed_since(close_started, close_cpu_started);
    let common = |metric: Metric| {
        metric
            .count("observations", args.profile_burst as u64)
            .count("hold_ms", hold_ms)
            .count("live_observations", args.profile_burst as u64)
    };
    Ok(vec![
        common(
            Metric::new(
                "public_facade",
                "live_profile_cohort_open",
                "per_identity:redb:live",
                open_elapsed,
                open_samples,
            )
            .cpu(open_cpu)
            .note("unlimited kind:0 observations retained across the 10ms grouping window"),
        ),
        common(
            Metric::new(
                "public_facade",
                "live_profile_cohort_hold",
                "per_identity:redb:live",
                hold_elapsed,
                Samples::default(),
            )
            .cpu(hold_cpu),
        ),
        common(
            Metric::new(
                "public_facade",
                "live_profile_cohort_close",
                "per_identity:redb:live",
                close_elapsed,
                close_samples,
            )
            .cpu(close_cpu)
            .note("cancel every independent profile observation and shut down the engine"),
        ),
    ])
}

fn config(store_path: &Path) -> EngineConfig {
    EngineConfig {
        store_path: Some(store_path.to_string_lossy().into_owned()),
        ..EngineConfig::default()
    }
}
