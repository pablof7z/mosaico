use super::Nip29Provider;
use anyhow::Context;
use anyhow::Result;
use std::collections::BTreeSet;
use std::future::Future;
use std::time::Duration;
use tokio::sync::watch;

const RELATIONSHIP_READBACK_TIMEOUT: Duration = Duration::from_secs(15);

/// Re-read pacing for a parent outside the view's group scope.
///
/// The view drops out-of-scope groups, so no group change will ever announce
/// the parent's new metadata; its one-shot relay read is re-polled on this
/// doubling schedule instead, bounded by `RELATIONSHIP_READBACK_TIMEOUT`.
struct OutOfScopeBackoff {
    next: Duration,
}

impl OutOfScopeBackoff {
    const INITIAL: Duration = Duration::from_millis(500);
    const MAX: Duration = Duration::from_secs(2);

    fn new() -> Self {
        Self {
            next: Self::INITIAL,
        }
    }

    fn next_delay(&mut self) -> Duration {
        let delay = self.next;
        self.next = (delay * 2).min(Self::MAX);
        delay
    }
}

/// What wakes the next parent re-read during relationship readback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ParentWake {
    /// The parent is in the view's group scope: a group change announces it.
    GroupChange,
    /// The view drops the parent, so only a timed re-read can see it.
    Backoff,
}

impl Nip29Provider {
    /// An in-view parent is re-read on group changes; any other on backoff.
    fn parent_wake(&self, parent_h: &str) -> ParentWake {
        if self.current_group_snapshot(parent_h).is_some() {
            ParentWake::GroupChange
        } else {
            ParentWake::Backoff
        }
    }

    async fn fetch_parent_children(&self, parent_h: &str) -> Result<BTreeSet<String>> {
        let snapshot = self.group_snapshot(parent_h).await?;
        Ok(snapshot
            .metadata
            .as_ref()
            .map(|metadata| {
                metadata
                    .tags
                    .iter()
                    .filter_map(|tag| {
                        (tag.first().map(String::as_str) == Some("child"))
                            .then(|| tag.get(1).cloned())
                            .flatten()
                            .filter(|child| !child.is_empty())
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Wait until the relay's parent metadata reciprocally confirms `child_h`.
    ///
    /// Croissant derives this reverse projection from the accepted child 9007;
    /// clients only verify relay truth and never race replacement-style parent
    /// metadata writes of their own.
    pub(in crate::fabric::provider) async fn confirm_parent_lists_child(
        &self,
        parent_h: &str,
        child_h: &str,
    ) -> Result<()> {
        let changes = self.view.subscribe_group_changes();
        let readback = await_parent_listing(
            || self.fetch_parent_children(parent_h),
            || self.parent_wake(parent_h),
            changes,
            parent_h,
            child_h,
        );
        tokio::time::timeout(RELATIONSHIP_READBACK_TIMEOUT, readback)
            .await
            .with_context(|| {
                format!("relay did not confirm child {child_h:?} in parent metadata")
            })?
    }
}

/// Re-read the parent until it lists `child_h`. An in-scope parent is re-read
/// on each group change; an out-of-scope one also on `OutOfScopeBackoff`.
async fn await_parent_listing<R, F>(
    mut read: R,
    wake: impl Fn() -> ParentWake,
    mut changes: watch::Receiver<u64>,
    parent_h: &str,
    child_h: &str,
) -> Result<()>
where
    R: FnMut() -> F,
    F: Future<Output = Result<BTreeSet<String>>>,
{
    let mut backoff = OutOfScopeBackoff::new();
    loop {
        match read().await {
            Ok(children) if children.contains(child_h) => return Ok(()),
            Ok(_) => {}
            Err(error) => tracing::debug!(
                parent = parent_h,
                child = child_h,
                error = %format!("{error:#}"),
                "parent relationship read awaits a newer complete view"
            ),
        }
        if wake() == ParentWake::GroupChange {
            changes
                .changed()
                .await
                .context("NIP-29 view closed during parent relationship readback")?;
        } else {
            tokio::select! {
                changed = changes.changed() => {
                    changed.context("NIP-29 view closed during parent relationship readback")?;
                }
                () = tokio::time::sleep(backoff.next_delay()) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn out_of_scope_backoff_doubles_to_its_cap() {
        let mut backoff = OutOfScopeBackoff::new();
        let delays: Vec<_> = (0..4).map(|_| backoff.next_delay()).collect();
        assert_eq!(
            delays,
            [
                OutOfScopeBackoff::INITIAL,
                OutOfScopeBackoff::INITIAL * 2,
                OutOfScopeBackoff::MAX,
                OutOfScopeBackoff::MAX,
            ]
        );
    }

    fn read_listing_after(
        reads: &Cell<u32>,
        listed_from: u32,
    ) -> impl FnMut() -> std::future::Ready<Result<BTreeSet<String>>> + '_ {
        move || {
            reads.set(reads.get() + 1);
            std::future::ready(Ok(if reads.get() >= listed_from {
                BTreeSet::from(["child".to_string()])
            } else {
                BTreeSet::new()
            }))
        }
    }

    #[tokio::test(start_paused = true)]
    async fn out_of_scope_parent_is_re_read_without_a_group_change() {
        let (_sender, changes) = watch::channel(0_u64);
        let reads = Cell::new(0);

        tokio::time::timeout(
            OutOfScopeBackoff::INITIAL * 2,
            await_parent_listing(
                read_listing_after(&reads, 2),
                || ParentWake::Backoff,
                changes,
                "parent",
                "child",
            ),
        )
        .await
        .expect("an out-of-scope parent must be re-read on backoff, not wait for a group change")
        .unwrap();
        assert_eq!(reads.get(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn in_scope_parent_waits_for_a_group_change_without_polling() {
        let (sender, changes) = watch::channel(0_u64);
        let reads = Cell::new(0);
        let readback = await_parent_listing(
            read_listing_after(&reads, 2),
            || ParentWake::GroupChange,
            changes,
            "parent",
            "child",
        );
        tokio::pin!(readback);

        let idle = tokio::time::timeout(RELATIONSHIP_READBACK_TIMEOUT, &mut readback).await;
        assert!(
            idle.is_err(),
            "an in-scope parent must not be re-read by timer"
        );
        assert_eq!(reads.get(), 1);

        sender.send_modify(|revision| *revision += 1);
        readback.await.unwrap();
        assert_eq!(reads.get(), 2);
    }

    #[tokio::test]
    async fn parent_wake_polls_only_a_parent_missing_from_the_view() {
        let state = crate::daemon::server::DaemonState::new_for_test().await;
        let provider = state.provider();
        assert_eq!(provider.parent_wake("parent"), ParentWake::Backoff);

        use nostr::prelude::{EventBuilder, FinalizeEvent, Keys, Kind, Tag};
        let relay = Keys::generate();
        let metadata = EventBuilder::new(Kind::Custom(39_000), "")
            .tags([Tag::custom("d", ["parent"])])
            .finalize(&relay)
            .unwrap();
        let view = state.nip29_view();
        view.replace_group_scope(BTreeSet::from(["parent".to_string()]), BTreeSet::new());
        view.replace(vec![metadata]);

        assert_eq!(provider.parent_wake("parent"), ParentWake::GroupChange);
    }
}
