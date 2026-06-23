//! Daemon integration: state-mutating RPCs end-to-end, multi-agent mention
//! routing, and the ~16-concurrent-writer corruption repro through the RPC path.
//!
//! All tests run against a real spawned `__daemon` (one relay → a local
//! `nak serve`, never the production fabric) over a UDS in an isolated
//! `TENEX_EDGE_HOME`. Env mutation is serialized; the file is run single-threaded
//! by the runner invocation in the SUMMARY (each test sets process-global env).

mod common;
#[path = "daemon_integration/harness.rs"]
mod daemon_harness;
#[path = "daemon_integration/freeze.rs"]
mod freeze;
#[path = "daemon_integration/channels.rs"]
mod channels;
#[path = "daemon_integration/messaging.rs"]
mod messaging;
#[path = "daemon_integration/process.rs"]
mod process;
