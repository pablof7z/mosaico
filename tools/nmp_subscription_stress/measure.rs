use std::collections::BTreeMap;
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub(crate) struct Samples {
    values: Vec<Duration>,
}

impl Samples {
    pub(crate) fn record<T>(&mut self, operation: impl FnOnce() -> T) -> T {
        let started = Instant::now();
        let result = operation();
        self.values.push(started.elapsed());
        result
    }

    pub(crate) fn push(&mut self, value: Duration) {
        self.values.push(value);
    }

    pub(crate) fn len(&self) -> usize {
        self.values.len()
    }

    fn percentile(&self, numerator: usize, denominator: usize) -> Duration {
        if self.values.is_empty() {
            return Duration::ZERO;
        }
        let mut sorted = self.values.clone();
        sorted.sort_unstable();
        sorted[(sorted.len() - 1) * numerator / denominator]
    }

    pub(crate) fn p50(&self) -> Duration {
        self.percentile(50, 100)
    }

    pub(crate) fn p95(&self) -> Duration {
        self.percentile(95, 100)
    }
}

#[derive(Debug)]
pub(crate) struct Metric {
    pub(crate) boundary: &'static str,
    pub(crate) phase: &'static str,
    pub(crate) topology: String,
    pub(crate) operations: usize,
    pub(crate) elapsed: Duration,
    pub(crate) samples: Samples,
    pub(crate) cpu: Option<Duration>,
    pub(crate) counts: BTreeMap<&'static str, u64>,
    pub(crate) status: &'static str,
    pub(crate) note: &'static str,
}

impl Metric {
    pub(crate) fn new(
        boundary: &'static str,
        phase: &'static str,
        topology: impl Into<String>,
        elapsed: Duration,
        samples: Samples,
    ) -> Self {
        let operations = samples.len();
        Self {
            boundary,
            phase,
            topology: topology.into(),
            operations,
            elapsed,
            samples,
            cpu: None,
            counts: BTreeMap::new(),
            status: "measured",
            note: "",
        }
    }

    pub(crate) fn count(mut self, name: &'static str, value: u64) -> Self {
        self.counts.insert(name, value);
        self
    }

    pub(crate) fn cpu(mut self, value: Duration) -> Self {
        self.cpu = Some(value);
        self
    }

    pub(crate) fn contract_status(mut self, satisfied: bool) -> Self {
        self.status = if satisfied {
            "contract_pass"
        } else {
            "known_red"
        };
        self
    }

    pub(crate) fn unavailable(mut self) -> Self {
        self.status = "unavailable";
        self
    }

    pub(crate) fn known_red_safe_full_b(mut self) -> Self {
        self.status = "known_red_safe_full_b";
        self
    }

    pub(crate) fn note(mut self, value: &'static str) -> Self {
        self.note = value;
        self
    }

    pub(crate) fn throughput(&self) -> f64 {
        if self.elapsed.is_zero() {
            return 0.0;
        }
        self.operations as f64 / self.elapsed.as_secs_f64()
    }
}

pub(crate) fn process_cpu_time() -> Duration {
    let mut value = std::mem::MaybeUninit::<libc::timespec>::uninit();
    // SAFETY: a successful clock_gettime initializes the owned timespec.
    let result = unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, value.as_mut_ptr()) };
    if result != 0 {
        return Duration::ZERO;
    }
    // SAFETY: the successful call above initialized value.
    let value = unsafe { value.assume_init() };
    Duration::new(value.tv_sec.max(0) as u64, value.tv_nsec.max(0) as u32)
}

pub(crate) fn elapsed_since(started: Instant, cpu_started: Duration) -> (Duration, Duration) {
    (
        started.elapsed(),
        process_cpu_time().saturating_sub(cpu_started),
    )
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ResourceSnapshot {
    pub(crate) open_fds: u64,
    pub(crate) current_rss_bytes: u64,
    pub(crate) physical_footprint_bytes: u64,
    pub(crate) peak_rss_bytes: u64,
    pub(crate) nmp_threads_live: u64,
}

pub(crate) fn resources() -> ResourceSnapshot {
    ResourceSnapshot {
        open_fds: open_fd_count(),
        current_rss_bytes: current_memory().0,
        physical_footprint_bytes: current_memory().1,
        peak_rss_bytes: peak_rss_bytes(),
        nmp_threads_live: nmp::nmp_threads_live(),
    }
}

#[cfg(target_os = "macos")]
fn current_memory() -> (u64, u64) {
    let mut usage = std::mem::MaybeUninit::<libc::rusage_info_v2>::uninit();
    let result = unsafe {
        libc::proc_pid_rusage(
            std::process::id() as libc::c_int,
            libc::RUSAGE_INFO_V2,
            usage.as_mut_ptr().cast(),
        )
    };
    if result < 0 {
        return (0, 0);
    }
    let usage = unsafe { usage.assume_init() };
    (usage.ri_resident_size, usage.ri_phys_footprint)
}

#[cfg(not(target_os = "macos"))]
fn current_memory() -> (u64, u64) {
    (0, 0)
}

fn open_fd_count() -> u64 {
    // The harness samples this only at phase boundaries. Scanning the finite
    // descriptor table avoids platform-specific /proc assumptions on macOS.
    let ceiling = unsafe { libc::getdtablesize() }.max(0);
    (0..ceiling)
        .filter(|fd| unsafe { libc::fcntl(*fd, libc::F_GETFD) } != -1)
        .count() as u64
}

fn peak_rss_bytes() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return 0;
    }
    let raw = unsafe { usage.assume_init() }.ru_maxrss.max(0) as u64;
    #[cfg(target_os = "linux")]
    {
        raw.saturating_mul(1_024)
    }
    #[cfg(not(target_os = "linux"))]
    {
        raw
    }
}
