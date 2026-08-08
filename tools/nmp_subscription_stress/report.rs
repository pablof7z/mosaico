use std::fmt::Write as _;

use crate::args::{Args, OutputFormat};
use crate::measure::Metric;

pub(crate) struct Reporter {
    format: OutputFormat,
}

impl Reporter {
    pub(crate) fn new(args: &Args) -> Self {
        match args.format {
            OutputFormat::Human => {
                println!(
                    "nmp_subscription_stress seed={} retained={} mailboxes={} profiles={} corpus={} iterations={}",
                    args.seed,
                    args.retained,
                    args.mailboxes,
                    args.profile_burst,
                    args.corpus_rows,
                    args.iterations
                );
                println!("nmp_rev=bca64d75eeee8496b93ca220976c4fa6046cf6cb network=disabled store=temporary");
            }
            OutputFormat::Csv => println!(
                "boundary,phase,topology,operations,elapsed_ms,throughput_per_s,p50_ms,p95_ms,cpu_ms,counts,note"
            ),
        }
        Self {
            format: args.format,
        }
    }

    pub(crate) fn metric(&self, metric: &Metric) {
        match self.format {
            OutputFormat::Human => print_human(metric),
            OutputFormat::Csv => print_csv(metric),
        }
    }

    pub(crate) fn limitations(&self) {
        if self.format == OutputFormat::Csv {
            return;
        }
        println!("\nAttribution boundaries:");
        println!("  public_facade = supported Engine::observe/drop/shutdown path");
        println!(
            "  internal_control = opt-in NMP reducer/store/router benchmark seams, never product behavior"
        );
        println!(
            "  NMP exposes exact Redb event-row and coverage-read counters, but not router sub-phase timers."
        );
        println!("  Headless core measures reducer integration; direct store/coalescer/diff rows isolate lower bounds.");
        println!("  Evidence-only source-status recomputation needs a scripted transport and is not measured here.");
    }
}

fn print_human(metric: &Metric) {
    let mut counts = String::new();
    for (key, value) in &metric.counts {
        let _ = write!(counts, " {key}={value}");
    }
    let cpu = metric
        .cpu
        .map(|value| format!(" cpu_ms={:.3}", ms(value)))
        .unwrap_or_default();
    println!(
        "{:<16} {:<26} topology={:<12} ops={:<6} elapsed_ms={:>9.3} rate_s={:>10.1} p50_ms={:>8.3} p95_ms={:>8.3}{}{}",
        metric.boundary,
        metric.phase,
        metric.topology,
        metric.operations,
        ms(metric.elapsed),
        metric.throughput(),
        ms(metric.samples.p50()),
        ms(metric.samples.p95()),
        cpu,
        counts,
    );
    if !metric.note.is_empty() {
        println!("  {}", metric.note);
    }
}

fn print_csv(metric: &Metric) {
    let counts = metric
        .counts
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(";");
    println!(
        "{},{},{},{},{:.6},{:.3},{:.6},{:.6},{},{},{}",
        csv(metric.boundary),
        csv(metric.phase),
        csv(&metric.topology),
        metric.operations,
        ms(metric.elapsed),
        metric.throughput(),
        ms(metric.samples.p50()),
        ms(metric.samples.p95()),
        metric
            .cpu
            .map(|value| format!("{:.6}", ms(value)))
            .unwrap_or_default(),
        csv(&counts),
        csv(metric.note)
    );
}

fn ms(duration: std::time::Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn csv(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
