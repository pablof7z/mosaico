use serde::{Deserialize, Serialize};

/// How Mosaico drives one canonical harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Transport {
    Pty,
    Acp,
    AppServer,
    PiRpc,
}

impl Transport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pty => "pty",
            Self::Acp => "acp",
            Self::AppServer => "app-server",
            Self::PiRpc => "pi-rpc",
        }
    }
}
