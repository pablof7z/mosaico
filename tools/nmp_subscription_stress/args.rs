use anyhow::{bail, Result};
use clap::{Parser, ValueEnum};

/// Hermetic stress controls. Defaults approximate the captured Mosaico shape.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "nmp-subscription-stress",
    about = "Attribute NMP/Mosaico subscription CPU without a relay or live state",
    after_help = "Examples:\n  just stress-nmp\n  just stress-nmp --scenario router --topology both --iterations 20\n  just stress-nmp --format csv > stress.csv"
)]
pub(crate) struct Args {
    /// Which attribution boundary to exercise.
    #[arg(long, value_enum, default_value_t = Scenario::Captured)]
    pub(crate) scenario: Scenario,

    /// Compare one observation per value, sharded set-valued observations, or both.
    #[arg(long, value_enum, default_value_t = TopologyChoice::Both)]
    pub(crate) topology: TopologyChoice,

    /// Output compact human-readable rows or stable CSV columns.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    pub(crate) format: OutputFormat,

    /// Total long-lived semantic watches (180 mailboxes plus 27 other by default).
    #[arg(long, default_value_t = 207)]
    pub(crate) retained: usize,

    /// Retained kind:9 #p mailbox watches, each for one identity.
    #[arg(long, default_value_t = 180)]
    pub(crate) mailboxes: usize,

    /// Number of short-lived, windowed kind:0 profile lookups.
    #[arg(long, default_value_t = 64)]
    pub(crate) profile_burst: usize,

    /// Maximum simultaneous short-lived profile lookup workers.
    #[arg(long, default_value_t = 16)]
    pub(crate) burst_size: usize,

    /// Values carried by each set-valued observation in sharded mode.
    #[arg(long, default_value_t = 64)]
    pub(crate) shard_size: usize,

    /// Deterministic corpus rows inserted into the disposable redb store.
    #[arg(long, default_value_t = 2_000)]
    pub(crate) corpus_rows: usize,

    /// Repetitions for direct store/router controls.
    #[arg(long, default_value_t = 5)]
    pub(crate) iterations: usize,

    /// Stable standing-observation hold interval used for wall/CPU comparison.
    #[arg(long, default_value_t = 100)]
    pub(crate) hold_ms: u64,

    /// Fixed deterministic fixture seed; it never selects network or secrets.
    #[arg(long, default_value_t = 29)]
    pub(crate) seed: u64,

    /// Logical-demand relationship used by the lifecycle matrix.
    #[arg(long, value_enum, default_value_t = DemandShape::All)]
    pub(crate) demand_shape: DemandShape,

    /// Open/close schedule used by the lifecycle matrix.
    #[arg(long, value_enum, default_value_t = LifecycleSchedule::All)]
    pub(crate) lifecycle_schedule: LifecycleSchedule,

    /// Observation counts exercised by the lifecycle matrix.
    #[arg(
        long,
        value_delimiter = ',',
        default_value = "1,32,207,1000,4096,10000"
    )]
    pub(crate) matrix_counts: Vec<usize>,
}

impl Args {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.retained == 0 {
            bail!("--retained must be greater than zero");
        }
        if self.mailboxes > self.retained {
            bail!("--mailboxes cannot exceed --retained");
        }
        if self.retained > 10_000 {
            bail!("--retained is capped at 10000 for a safe local run");
        }
        if self.burst_size == 0 || self.shard_size == 0 || self.iterations == 0 {
            bail!("--burst-size, --shard-size, and --iterations must be non-zero");
        }
        if self.profile_burst > 4_096 || self.burst_size > 1_024 {
            bail!("profile lookup counts exceed the safe local ceiling");
        }
        if self.scenario.includes(Scenario::Consumer) && self.retained > 1_024 {
            bail!("consumer drain-thread runs are capped at 1024; use matrix or facade for larger NMP loads");
        }
        if self
            .matrix_counts
            .iter()
            .any(|count| *count == 0 || *count > 10_000)
        {
            bail!("--matrix-counts values must be between 1 and 10000");
        }
        let fixture_identities = self
            .retained
            .max(self.mailboxes)
            .max(self.profile_burst)
            .max(1);
        let needs_populated_store = matches!(
            self.scenario,
            Scenario::Captured
                | Scenario::Facade
                | Scenario::Store
                | Scenario::Profiles
                | Scenario::Freshness
        );
        if needs_populated_store && self.corpus_rows < fixture_identities {
            bail!(
                "--corpus-rows must be at least {fixture_identities} to seed one profile per fixture identity"
            );
        }
        Ok(())
    }

    pub(crate) fn topologies(&self) -> &'static [Topology] {
        match self.topology {
            TopologyChoice::PerIdentity => &[Topology::PerIdentity],
            TopologyChoice::Sharded => &[Topology::Sharded],
            TopologyChoice::Both => &[Topology::PerIdentity, Topology::Sharded],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Scenario {
    Captured,
    Facade,
    Store,
    Router,
    Consumer,
    Profiles,
    Freshness,
    Matrix,
}

impl Scenario {
    pub(crate) fn includes(self, phase: Self) -> bool {
        self == phase
            || (self == Self::Captured && !matches!(phase, Self::Freshness | Self::Matrix))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum TopologyChoice {
    PerIdentity,
    Sharded,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Topology {
    PerIdentity,
    Sharded,
}

impl Topology {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::PerIdentity => "per_identity",
            Self::Sharded => "sharded",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum OutputFormat {
    Human,
    Csv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum DemandShape {
    All,
    ExactDuplicate,
    CompatibleDistinct,
    ProfileAuthors,
    LimitedIncompatible,
    UnlimitedMultiAxisIncompatible,
}

impl DemandShape {
    pub(crate) fn selected(self) -> &'static [Self] {
        match self {
            Self::All => &[
                Self::ExactDuplicate,
                Self::CompatibleDistinct,
                Self::ProfileAuthors,
                Self::LimitedIncompatible,
                Self::UnlimitedMultiAxisIncompatible,
            ],
            Self::ExactDuplicate => &[Self::ExactDuplicate],
            Self::CompatibleDistinct => &[Self::CompatibleDistinct],
            Self::ProfileAuthors => &[Self::ProfileAuthors],
            Self::LimitedIncompatible => &[Self::LimitedIncompatible],
            Self::UnlimitedMultiAxisIncompatible => &[Self::UnlimitedMultiAxisIncompatible],
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::ExactDuplicate => "exact_duplicate",
            Self::CompatibleDistinct => "compatible_distinct",
            Self::ProfileAuthors => "profile_authors",
            Self::LimitedIncompatible => "limited_incompatible",
            Self::UnlimitedMultiAxisIncompatible => "unlimited_multi_axis_incompatible",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum LifecycleSchedule {
    All,
    Forward,
    Reverse,
    SeededRandom,
    BeforeAdmission,
    Interleaved,
}

impl LifecycleSchedule {
    pub(crate) fn selected(self) -> &'static [Self] {
        match self {
            Self::All => &[
                Self::Forward,
                Self::Reverse,
                Self::SeededRandom,
                Self::BeforeAdmission,
                Self::Interleaved,
            ],
            Self::Forward => &[Self::Forward],
            Self::Reverse => &[Self::Reverse],
            Self::SeededRandom => &[Self::SeededRandom],
            Self::BeforeAdmission => &[Self::BeforeAdmission],
            Self::Interleaved => &[Self::Interleaved],
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Forward => "forward",
            Self::Reverse => "reverse",
            Self::SeededRandom => "seeded_random",
            Self::BeforeAdmission => "before_admission",
            Self::Interleaved => "interleaved",
        }
    }
}

#[cfg(test)]
#[path = "args/tests.rs"]
mod tests;
