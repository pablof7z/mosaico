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
    #[arg(long, value_enum, default_value_t = Scenario::All)]
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
}

impl Args {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.retained == 0 {
            bail!("--retained must be greater than zero");
        }
        if self.mailboxes > self.retained {
            bail!("--mailboxes cannot exceed --retained");
        }
        if self.retained > 4_096 {
            bail!("--retained is capped at 4096 for a safe local run");
        }
        if self.burst_size == 0 || self.shard_size == 0 || self.iterations == 0 {
            bail!("--burst-size, --shard-size, and --iterations must be non-zero");
        }
        if self.profile_burst > 4_096 || self.burst_size > 1_024 {
            bail!("profile lookup counts exceed the safe local ceiling");
        }
        let fixture_identities = self.mailboxes.max(self.profile_burst).max(1);
        if self.corpus_rows < fixture_identities {
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
    All,
    Facade,
    Store,
    Router,
    Consumer,
    Profiles,
}

impl Scenario {
    pub(crate) fn includes(self, phase: Self) -> bool {
        self == Self::All || self == phase
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_shape_is_the_default() {
        let args = Args::parse_from(["stress"]);
        assert_eq!((args.retained, args.mailboxes), (207, 180));
        assert_eq!(args.topologies().len(), 2);
        args.validate().unwrap();
    }
}
