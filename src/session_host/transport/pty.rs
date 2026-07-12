//! `PtyTransport`: the PTY transport marker.
//!
//! The PTY launch/resume/deliver paths are driven DIRECTLY — `pty::spawn_session`
//! / `pty::inject` in `session_host::launch` / `session_host::delivery`, with
//! resume argv shaped by the `registry` argv sniffers — NOT through this type. So
//! the only methods ever reached are `kind` (endpoint classification) and `kill`
//! (endpoint rollback via [`super::TransportImpl::kill`]).
//!
//! The dead `SessionTransport` `launch`/`resume`/`deliver`/`is_live` methods that
//! once mirrored the direct path were removed. They were never called and one of
//! them (`launch`/`resume`) hardcoded `durable_reservation: None`, a latent trap
//! that silently dropped the supervisor's durable reservation had the trait path
//! ever gone live. `AcpTransport` remains the sole full [`super::SessionTransport`]
//! implementation; `PtyTransport` exposes only the two inherent methods the enum
//! dispatcher actually invokes.

use anyhow::Result;

use super::{EndpointRef, TransportKind};

pub struct PtyTransport;

impl PtyTransport {
    pub fn kind(&self) -> TransportKind {
        TransportKind::Pty
    }

    pub async fn kill(&self, ep: &EndpointRef) -> Result<()> {
        crate::pty::kill(&ep.endpoint_id)
    }
}
