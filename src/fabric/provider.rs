//! Phase 5: `Kind1Nip29Provider` — concrete provider wrapping delivery, wire
//! codec, materializer, and lifecycle in one place.
//!
//! Design constraints (see docs/fabric-architecture.md §Phase 5):
//! - CONCRETE struct with INHERENT async methods; no async_trait, no dyn.
//! - Single-writer invariant: the provider holds a CLONE of the same
//!   `Arc<Mutex<Store>>` that `DaemonState` owns — one SQLite connection total.
//! - Dynamic per-call data (the hosted "me" set, owners, now) is passed to
//!   methods, not stored; the provider owns only stable construction-time data.
//!
//! Phase 6 adds `SendIntent`, `OutboundReceipt`, and `provider.send()` for
//! canonical dual-write outbound messages alongside the legacy inbox path.

use crate::domain::{AgentRef, DomainEvent, Mention};
use crate::fabric::kind1::wire::Kind1WireCodec;
use crate::fabric::nostr_delivery::NostrDelivery;
use crate::fabric::{MaterializationOutcome, RawEnvelope, WireCodec};
use crate::state::{SessionRecord, Store};
use crate::transport::Transport;
use crate::util::now_secs;
use anyhow::Result;
use nostr_sdk::EventBuilder;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// Fabric identifier used in all canonical origin rows.
pub const FABRIC: &str = "kind1-nip29";

// ── Phase 6 send types ────────────────────────────────────────────────────────

/// All inputs needed to publish one outbound message.
pub struct SendIntent {
    /// Sender's identity on the fabric.
    pub from: AgentRef,
    /// Recipient's pubkey (hex). Stage 4: always the session pubkey (or agent
    /// pubkey when no session key was derived), never the legacy agent pubkey
    /// + session-id pair.
    pub to_pubkey: String,
    /// Project slug (NIP-29 group name).
    pub project: String,
    /// Message body.
    pub body: String,
    /// Existing canonical thread id to attach to. `None` → a new thread root
    /// is created from the published event id (Phase 7 will refine).
    pub thread_id: Option<String>,
    /// Envelope metadata (subject + sender workspace snapshot + reply-to).
    /// Rides the wire as tags; rendered as an email-like header on receipt.
    pub meta: crate::domain::MentionMeta,
}

impl SendIntent {
    /// Convert to the `Mention` domain event used by the wire codec and the
    /// local-delivery path. Both callers see the same struct.
    pub fn to_mention(&self) -> Mention {
        Mention {
            from: self.from.clone(),
            to_pubkey: self.to_pubkey.clone(),
            project: self.project.clone(),
            body: self.body.clone(),
            meta: self.meta.clone(),
        }
    }
}

/// Result of a successful `provider.send()`.
pub struct OutboundReceipt {
    /// Hex event id of the published Nostr event.
    pub native_event_id: String,
    /// Canonical `message_id` inserted into the read-model `messages` table.
    pub message_id: String,
    /// Sync state stored on the canonical message (`"published"` or `"accepted"`).
    pub sync_state: String,
    /// The canonical thread_id the message was filed under (new or existing).
    pub thread_id: String,
}

// ── Trait shell (documentation only; daemon calls concrete inherent methods) ───

/// Shell trait documenting the provider API surface.
///
/// Phase 5 implements the concrete methods directly on `Kind1Nip29Provider`
/// (inherent) rather than via `impl FabricProvider`, to avoid async-fn-in-trait
/// machinery. `send` and `set_status` are implemented as inherent methods.
#[allow(dead_code)]
pub trait FabricProvider {
    fn name(&self) -> &'static str;
    // async fn open_project(&self, project: &str, agent_pubkey: &str);
    // async fn send(&self, intent: SendIntent) -> Result<OutboundReceipt>;
    // async fn set_status(&self, status: &Status, keys: &Keys) -> Result<EventId>;
    // async fn subscribe_project(&self, scope: crate::fabric::Scope) -> Result<()>;
    // async fn catch_up_mentions(&self, rec: &SessionRecord) -> Result<usize>;
    // fn materialize(&self, env: RawEnvelope, hosted: &[String], owners: &[String], now: u64, store: &Store) -> MaterializationOutcome;
}

// ── Kind1Nip29Provider ────────────────────────────────────────────────────────

/// The first concrete provider: NIP-29 groups over Nostr kind:1 wire encoding.
///
/// Fields held at construction time (stable config). Per-call dynamic data
/// (hosted "me" set, owners, now) is received as method parameters.
pub struct Kind1Nip29Provider {
    pub delivery: NostrDelivery,
    pub wire: Kind1WireCodec,
    /// Shared store Arc — same handle as `DaemonState.store`. No new Connection.
    pub store: Arc<Mutex<Store>>,
    /// Same Arc as `DaemonState.transport` — used for lifecycle publishes only.
    pub transport: Arc<Transport>,
    /// Operator nsec for NIP-29 group management. Optional: if unset, group
    /// management is skipped and sessions still start (best-effort).
    pub user_nsec: Option<String>,
    /// Whitelisted human pubkeys (hex) from config. Every owned NIP-29 group
    /// grants each of these the `admin` role, backfilled on every `open_project`
    /// by diffing against the relay's live admin set.
    pub whitelisted_pubkeys: Vec<String>,
    /// Stable hash of the sorted relay URL set. Used as the `provider_instance`
    /// column in canonical origin rows, making them deterministic across daemon
    /// restarts. Derived once at construction from `cfg.relays`.
    pub provider_instance: String,
}

impl Kind1Nip29Provider {
    pub fn new(
        transport: Arc<Transport>,
        store: Arc<Mutex<Store>>,
        user_nsec: Option<String>,
        whitelisted_pubkeys: Vec<String>,
        relays: &[String],
    ) -> Self {
        let delivery = NostrDelivery::new(transport.clone());
        let wire = Kind1WireCodec;
        let provider_instance = derive_provider_instance(relays);
        Self {
            delivery,
            wire,
            store,
            transport,
            user_nsec,
            whitelisted_pubkeys,
            provider_instance,
        }
    }

    // ── name ──────────────────────────────────────────────────────────────────

    pub fn name(&self) -> &'static str {
        "kind1-nip29"
    }

    // ── encode / decode ───────────────────────────────────────────────────────

    /// Encode a domain event to an `EventBuilder` via the Kind1 wire codec.
    pub fn encode(&self, ev: &DomainEvent) -> Result<EventBuilder> {
        self.wire.encode(ev)
    }

    /// Decode a raw envelope to a domain event via the Kind1 wire codec.
    pub fn decode(&self, env: &RawEnvelope) -> Option<DomainEvent> {
        self.wire.decode(env)
    }

    /// Encode + sign + publish ONE domain event. The single wire-publish entry
    /// for everything above the seam (session engine liveness, turn replies,
    /// user prompts, proposals). Nothing above the provider builds an event.
    pub async fn publish(
        &self,
        ev: &DomainEvent,
        keys: &nostr_sdk::prelude::Keys,
    ) -> Result<nostr_sdk::prelude::EventId> {
        let builder = self.wire.encode(ev)?;
        self.transport.publish_signed(builder, keys).await
    }

    /// Like [`publish`], but FAILS when no relay accepted the event. Use for
    /// artifacts where a "published" report must mean the event is actually on
    /// the relay (e.g. kind:30023 proposals) — the bare [`publish`] reports an
    /// optimistic write-side ack and would hide a NIP-29 `blocked` rejection.
    pub async fn publish_checked(
        &self,
        ev: &DomainEvent,
        keys: &nostr_sdk::prelude::Keys,
    ) -> Result<nostr_sdk::prelude::EventId> {
        let builder = self.wire.encode(ev)?;
        self.transport.publish_signed_checked(builder, keys).await
    }

    /// Read an event back by id to confirm it is actually retrievable from the
    /// relay (reads on relay29's `closed`+`public` groups are open, so this works
    /// on the daemon's non-member connection). Returns `true` iff the relay
    /// returned the event within `timeout`.
    pub async fn is_retrievable(
        &self,
        id: nostr_sdk::prelude::EventId,
        timeout: std::time::Duration,
    ) -> bool {
        use nostr_sdk::prelude::Filter;
        let f = Filter::new().id(id).limit(1);
        self.transport
            .fetch(f, timeout)
            .await
            .map(|evs| !evs.is_empty())
            .unwrap_or(false)
    }

    /// Publish a mention that the daemon is emitting on behalf of `suppress_for`'s
    /// OWN session (a local user-prompt), pre-recording it as seen for that agent
    /// BEFORE the wire send so its relay echo is never routed back into that
    /// agent's own inbox. Marking seen before publishing closes the race where the
    /// echo arrives and routes before a post-publish mark could land. Returns the
    /// published event id. (Events are built/signed only here, never above the seam.)
    pub async fn publish_seen_by(
        &self,
        ev: &DomainEvent,
        keys: &nostr_sdk::prelude::Keys,
        suppress_for: &str,
    ) -> Result<nostr_sdk::prelude::EventId> {
        let builder = self.wire.encode(ev)?;
        let signed = self.transport.sign(builder, keys).await?;
        let eid = signed.id.to_hex();
        self.with_store(|s| s.mark_mention_seen(suppress_for, &eid, now_secs()).ok());
        self.transport.publish_event(&signed).await
    }

    /// Connectivity probe: publish a uniquely-tagged throwaway note on the
    /// daemon's connection key and read it back. Returns
    /// `(publish_result, readback_result)` as display strings — the wire shape
    /// of the probe is a provider detail.
    pub async fn doctor_probe(&self) -> (String, String) {
        use nostr_sdk::prelude::{Alphabet, EventBuilder, Filter, Kind, SingleLetterTag, Tag};
        let t = format!("te-doctor-{}", crate::util::now_secs());
        let publish = async {
            let builder = EventBuilder::new(Kind::from(1u16), format!("tenex-edge doctor {t}"))
                .tags([Tag::parse(["h", &t])?]);
            // Checked: assert the relay actually returned OK,true. The bare
            // publish reported a write-side ack, so a NIP-29 relay rejecting the
            // throwaway `#h` group still printed "publish: OK" — a false positive
            // that masked the very failure this probe exists to catch.
            self.transport.publish_builder_checked(builder).await
        }
        .await;
        let publish = match publish {
            Ok(id) => format!("OK ({})", crate::util::pubkey_short(&id.to_hex())),
            Err(e) => format!("ERR {e:#}"),
        };
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let f = Filter::new()
            .kind(Kind::from(1u16))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::H), &t)
            .limit(5);
        let readback = match self
            .transport
            .fetch(f, std::time::Duration::from_secs(5))
            .await
        {
            Ok(evs) => format!("{} event(s) with #h={t}", evs.len()),
            Err(e) => format!("ERR {e:#}"),
        };
        (publish, readback)
    }

    // ── group state ────────────────────────────────────────────────────────────

    /// Fetch the relay's LIVE state for `group`: `(exists, roles, members)`.
    /// `roles` maps pubkey → role from kind:39001 p-tags `["p", pk, role]`;
    /// `members` is the kind:39002 p-tag set. Keyed by `#d == group`. On fetch
    /// failure returns `(false, empty, empty)` so callers fail toward
    /// attempt-create rather than skip-assuming-it-exists.
    pub async fn fetch_group_state(
        &self,
        group: &str,
    ) -> (
        bool,
        std::collections::HashMap<String, String>,
        std::collections::HashSet<String>,
    ) {
        use crate::codec::kind1::{KIND_GROUP_ADMINS, KIND_GROUP_MEMBERS, KIND_GROUP_METADATA};
        use nostr_sdk::prelude::Filter;
        let filter = Filter::new()
            .kinds([
                crate::codec::kind1::kind(KIND_GROUP_METADATA),
                crate::codec::kind1::kind(KIND_GROUP_ADMINS),
                crate::codec::kind1::kind(KIND_GROUP_MEMBERS),
            ])
            .identifier(group);
        let state_evs = self
            .transport
            .fetch(filter, Duration::from_secs(5))
            .await
            .unwrap_or_default();

        // Newest event per kind (addressable replaceables; pick max created_at).
        let newest = |k: u16| {
            state_evs
                .iter()
                .filter(|e| e.kind.as_u16() == k)
                .max_by_key(|e| e.created_at.as_secs())
        };
        let group_exists = newest(KIND_GROUP_METADATA).is_some()
            || newest(KIND_GROUP_ADMINS).is_some()
            || newest(KIND_GROUP_MEMBERS).is_some();

        // Role map from 39001 p-tags: ["p", pubkey, role].
        let mut roles: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        if let Some(ev) = newest(KIND_GROUP_ADMINS) {
            for t in ev.tags.iter() {
                let s = t.as_slice();
                if s.first().map(String::as_str) == Some("p") {
                    if let Some(pk) = s.get(1) {
                        roles.insert(
                            pk.clone(),
                            s.get(2).cloned().unwrap_or_else(|| "member".to_string()),
                        );
                    }
                }
            }
        }
        // Member set from 39002 p-tags: ["p", pubkey].
        let mut members: std::collections::HashSet<String> = std::collections::HashSet::new();
        if let Some(ev) = newest(KIND_GROUP_MEMBERS) {
            for t in ev.tags.iter() {
                let s = t.as_slice();
                if s.first().map(String::as_str) == Some("p") {
                    if let Some(pk) = s.get(1) {
                        members.insert(pk.clone());
                    }
                }
            }
        }
        (group_exists, roles, members)
    }

    /// Convenience: just the role map (kind:39001) for `group`.
    pub async fn fetch_group_roles(
        &self,
        group: &str,
    ) -> std::collections::HashMap<String, String> {
        self.fetch_group_state(group).await.1
    }

    /// Subscribe to subgroup orchestration (kind:9) events p-tagged to this
    /// backend's identity, independent of any project. Narrow single-filter
    /// subscription (NOT the full project scope) so we don't pull a kind:9/kind:1
    /// firehose — only events addressed to `backend_pubkey` arrive.
    pub async fn subscribe_backend_orchestration(&self, backend_pubkey: &str) -> Result<()> {
        use nostr_sdk::prelude::{Filter, PublicKey};
        if let Ok(pk) = PublicKey::from_hex(backend_pubkey) {
            let f = Filter::new()
                .kind(crate::codec::kind1::kind(crate::codec::kind1::KIND_CHAT))
                .pubkey(pk);
            self.transport.subscribe(vec![f]).await?;
        }
        Ok(())
    }

    /// The `parent` group id declared in `group`'s relay-authored kind:39000
    /// metadata, if any. `None` when the group has no metadata yet (brand-new,
    /// not echoed) or carries no `parent` tag. Used to verify a subgroup actually
    /// belongs to its claimed parent before provisioning into it.
    pub async fn fetch_group_parent(&self, group: &str) -> Option<String> {
        use crate::codec::kind1::KIND_GROUP_METADATA;
        use nostr_sdk::prelude::Filter;
        let filter = Filter::new()
            .kind(crate::codec::kind1::kind(KIND_GROUP_METADATA))
            .identifier(group);
        let evs = self
            .transport
            .fetch(filter, Duration::from_secs(5))
            .await
            .unwrap_or_default();
        let newest = evs.iter().max_by_key(|e| e.created_at.as_secs())?;
        newest.tags.iter().find_map(|t| {
            let s = t.as_slice();
            if s.first().map(String::as_str) == Some("parent") {
                s.get(1).cloned()
            } else {
                None
            }
        })
    }

    // ── open_project ─────────────────────────────────────────────────────────

    /// Ensure a closed NIP-29 group exists for `project`, that every whitelisted
    /// human pubkey holds the `admin` role, and that `agent_pubkey` is a member.
    /// Best-effort: never blocks session start.
    ///
    /// Decisions are driven by the relay's LIVE group state (kinds 39000/39001/
    /// 39002 fetched by `#d == project`), not the local cache — so a re-run
    /// auto-detects a missing group or a whitelisted pubkey that isn't yet an
    /// admin and repairs it (the "backfill" property). The trailing
    /// `ensure_subscription` call remains at the call site (rpc_session_start /
    /// reconcile_sessions) to preserve the existing double-subscribe behavior.
    pub async fn open_project(&self, project: &str, agent_pubkey: &str) {
        self.open_project_with_progress(project, agent_pubkey, |_| {})
            .await;
    }

    pub async fn open_project_with_progress<F>(
        &self,
        project: &str,
        agent_pubkey: &str,
        mut progress: F,
    ) where
        F: FnMut(String),
    {
        use nostr_sdk::prelude::Keys;
        progress(format!("using project group {project}"));
        let nsec = match &self.user_nsec {
            Some(n) => n.clone(),
            None => {
                progress("userNsec unset; skipping group management".to_string());
                if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                    eprintln!(
                        "[daemon] userNsec unset; skipping NIP-29 group management for {project}"
                    );
                }
                return;
            }
        };
        let user_keys = match Keys::parse(&nsec) {
            Ok(k) => k,
            Err(e) => {
                progress("userNsec parse failed; skipping group management".to_string());
                eprintln!("[daemon] userNsec parse failed; skipping group management: {e}");
                return;
            }
        };

        // Query the RELAY (not the local cache) for the group's live state.
        progress("fetching relay group metadata/admins/members".to_string());
        let (group_exists, roles, members) = self.fetch_group_state(project).await;
        progress(format!(
            "relay group state: exists={group_exists}, {} role(s), {} member(s)",
            roles.len(),
            members.len()
        ));

        // 1. Create + lock the group if the relay has no record of it.
        if !group_exists {
            progress("group not found; publishing kind:9007 create-group".to_string());
            let created = match crate::fabric::nip29::lifecycle::group_create(project) {
                Ok(b) => {
                    self.publish_group_management(b, &user_keys, "9007 create-group")
                        .await
                }
                Err(_) => false,
            };
            progress(if created {
                "create-group accepted or already existed".to_string()
            } else {
                "create-group was not accepted; will retry on next session".to_string()
            });
            let locked = if created {
                progress("publishing kind:9002 closed/public group lock".to_string());
                match crate::fabric::nip29::lifecycle::group_lock_closed(project) {
                    Ok(b) => {
                        self.publish_group_management(b, &user_keys, "9002 lock-closed")
                            .await
                    }
                    Err(_) => false,
                }
            } else {
                false
            };
            if created {
                progress(if locked {
                    "group lock accepted or already existed".to_string()
                } else {
                    "group lock was not accepted; will retry on next session".to_string()
                });
            }
            if created && locked {
                self.with_store(|s| {
                    s.mark_group_owned(project, now_secs()).ok();
                });
            }
        } else {
            progress("group already exists on relay".to_string());
        }

        // 2. Backfill admins: every whitelisted human pubkey MUST hold the admin
        //    role. Diff against the relay's live 39001 set (above) so a re-run
        //    repairs any pubkey that is missing or only a plain member.
        for pk in &self.whitelisted_pubkeys {
            if roles.get(pk).map(String::as_str) == Some("admin") {
                continue;
            }
            progress(format!(
                "granting admin to owner {}",
                crate::util::pubkey_short(pk)
            ));
            let granted = match crate::fabric::nip29::lifecycle::group_put_admin(project, pk) {
                Ok(b) => {
                    self.publish_group_management(b, &user_keys, "9000 put-user (admin)")
                        .await
                }
                Err(_) => false,
            };
            if granted {
                progress(format!(
                    "admin grant accepted for {}",
                    crate::util::pubkey_short(pk)
                ));
                self.with_store(|s| {
                    s.upsert_group_member(project, pk, "admin", now_secs()).ok();
                });
            } else {
                progress(format!(
                    "admin grant rejected for {}",
                    crate::util::pubkey_short(pk)
                ));
                // The admin backfill is otherwise invisible; surface a rejection
                // unconditionally so a bad role/permission can't masquerade as
                // success (an empty/unauthorized result would silently no-op).
                eprintln!(
                    "[daemon] NIP-29 admin grant for {} in group {project} was NOT accepted by the relay",
                    crate::util::pubkey_short(pk)
                );
            }
        }

        // 3. Add this agent as a member if the relay's live roster lacks it.
        if !members.contains(agent_pubkey) && !roles.contains_key(agent_pubkey) {
            progress(format!(
                "adding agent {} as group member",
                crate::util::pubkey_short(agent_pubkey)
            ));
            let added = match crate::fabric::nip29::lifecycle::group_put_user(project, agent_pubkey)
            {
                Ok(b) => {
                    self.publish_group_management(b, &user_keys, "9000 put-user")
                        .await
                }
                Err(_) => false,
            };
            if added {
                progress(format!(
                    "agent membership accepted for {}",
                    crate::util::pubkey_short(agent_pubkey)
                ));
                self.with_store(|s| {
                    s.upsert_group_member(project, agent_pubkey, "member", now_secs())
                        .ok();
                });
            } else {
                progress(format!(
                    "agent membership rejected for {}; will retry on next session",
                    crate::util::pubkey_short(agent_pubkey)
                ));
            }
        } else {
            progress(format!(
                "agent {} is already in the relay roster",
                crate::util::pubkey_short(agent_pubkey)
            ));
        }
    }

    async fn publish_group_management(
        &self,
        builder: EventBuilder,
        keys: &nostr_sdk::prelude::Keys,
        label: &str,
    ) -> bool {
        match self.transport.publish_signed_checked(builder, keys).await {
            Ok(_) => true,
            Err(e) => {
                let benign = {
                    let s = e.to_string();
                    s.contains("already exists") || s.contains("duplicate")
                };
                if !benign && std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                    eprintln!(
                        "[daemon] NIP-29 {label} publish failed (will retry next session): {e:#}"
                    );
                }
                benign
            }
        }
    }

    // ── session member management (Stage 2 / Issue #2) ───────────────────────

    /// Parse the operator signing key from `user_nsec`. Returns `None` when
    /// the nsec is absent or malformed (same skip-if-unset pattern as
    /// `open_project`).
    fn parse_user_keys(&self) -> Option<nostr_sdk::prelude::Keys> {
        self.user_nsec
            .as_ref()
            .and_then(|n| nostr_sdk::prelude::Keys::parse(n).ok())
    }

    /// Admin-add `pubkey_hex` to `project` as a plain member (not admin).
    ///
    /// Best-effort: returns `true` when the relay accepted the 9000 event or
    /// treated it as a benign duplicate ("already exists"). Returns `false`
    /// when `user_nsec` is absent, malformed, or the relay rejected the event.
    pub async fn nip29_add_member(&self, project: &str, pubkey_hex: &str) -> bool {
        let Some(user_keys) = self.parse_user_keys() else {
            return false;
        };
        match crate::fabric::nip29::lifecycle::group_put_user(project, pubkey_hex) {
            Ok(b) => {
                self.publish_group_management(b, &user_keys, "9000 put-user (session)")
                    .await
            }
            Err(_) => false,
        }
    }

    /// Admin-add `pubkey_hex` to `project` with the `admin` role. Best-effort,
    /// same accept/benign-duplicate semantics as [`nip29_add_member`].
    pub async fn nip29_add_admin(&self, project: &str, pubkey_hex: &str) -> bool {
        let Some(user_keys) = self.parse_user_keys() else {
            return false;
        };
        match crate::fabric::nip29::lifecycle::group_put_admin(project, pubkey_hex) {
            Ok(b) => {
                self.publish_group_management(b, &user_keys, "9000 put-user (admin)")
                    .await
            }
            Err(_) => false,
        }
    }

    /// Create + lock a NIP-29 SUBGROUP: publish kind:9007 create-group for
    /// `child_h`, then kind:9002 edit-metadata locking it closed+public, naming
    /// it `name`, and recording the `parent_h` relationship. Returns `true` when
    /// both events were accepted (or benign-duplicate). Best-effort; signed with
    /// the management key.
    pub async fn nip29_create_subgroup(&self, child_h: &str, name: &str, parent_h: &str) -> bool {
        let Some(user_keys) = self.parse_user_keys() else {
            return false;
        };
        let created = match crate::fabric::nip29::lifecycle::group_create_subgroup(child_h) {
            Ok(b) => {
                self.publish_group_management(b, &user_keys, "9007 create-subgroup")
                    .await
            }
            Err(_) => false,
        };
        if !created {
            return false;
        }
        match crate::fabric::nip29::lifecycle::group_lock_closed_with_parent(child_h, name, parent_h)
        {
            Ok(b) => {
                self.publish_group_management(b, &user_keys, "9002 lock-with-parent")
                    .await
            }
            Err(_) => false,
        }
    }

    /// Admin-remove `pubkey_hex` from `project`.
    ///
    /// Best-effort: returns `true` when the relay accepted the 9001 event or
    /// treated it as benign. Returns `false` when `user_nsec` is absent /
    /// malformed or the relay rejected the event. Callers MUST mirror into the
    /// `group_members` cache regardless, since relay rejection of a remove for
    /// a non-member is benign (idempotent).
    pub async fn nip29_remove_member(&self, project: &str, pubkey_hex: &str) -> bool {
        let Some(user_keys) = self.parse_user_keys() else {
            return false;
        };
        match crate::fabric::nip29::lifecycle::group_remove_user(project, pubkey_hex) {
            Ok(b) => {
                self.publish_group_management(b, &user_keys, "9001 remove-user (session)")
                    .await
            }
            Err(_) => false,
        }
    }

    // ── subscribe ─────────────────────────────────────────────────────────────

    /// Forward a `Scope` subscription to the underlying delivery layer.
    /// Called by `ensure_subscription` / `resubscribe` in the daemon.
    pub async fn subscribe(&self, scope: crate::fabric::Scope) -> Result<()> {
        self.delivery.subscribe(scope).await
    }

    // ── set_status ─────────────────────────────────────────────────────────────

    /// Publish ONE kind:30315 status for a session. The single wire-publish entry
    /// for session status: the daemon's status-outbox drainer and the per-heartbeat
    /// liveness re-arm both build a `Status` (with `expires_at = now +
    /// STATUS_TTL_SECS`) and call this; nothing else encodes a status event.
    ///
    /// The codec turns `status.expires_at` into a NIP-40 `["expiration", ts]` tag,
    /// so liveness IS the freshness of the published event. Uses the optimistic
    /// `publish_signed` (write-side ack) since status is re-armed every heartbeat —
    /// a single dropped publish self-heals on the next beat.
    pub async fn set_status(
        &self,
        status: &crate::domain::Status,
        keys: &nostr_sdk::prelude::Keys,
    ) -> Result<nostr_sdk::prelude::EventId> {
        let builder = self.wire.encode(&DomainEvent::Status(status.clone()))?;
        self.transport.publish_signed(builder, keys).await
    }

    // ── send ──────────────────────────────────────────────────────────────────

    /// Publish one outbound message and dual-write canonical read-model rows.
    ///
    /// Behavior:
    /// 1. If `intent.thread_id` is Some, look up the thread's root native key
    ///    (BEFORE publish; no lock held across await).
    /// 2. Encode the intent as a Mention DomainEvent. When a root native key was
    ///    found, append a NIP-10 root `["e", <key>, "", "root"]` tag to the
    ///    EventBuilder so the recipient materializer groups the reply into the
    ///    same thread.  If the thread has no registered origin, publish without
    ///    the tag (safe degradation: reply becomes a new root on the recipient's
    ///    side).
    /// 3. Publish. On error, propagate immediately — no canonical row written.
    /// 4. On success, lock the store ONCE and write:
    ///    - `projects` / `project_origins` (idempotent)
    ///    - `threads` / `thread_origins` (idempotent, keyed by event id for new
    ///      roots, by existing thread_id for replies)
    ///    - `messages` with `sync_state="published"` (idempotent on native_event_id)
    ///    - `message_recipients`
    /// 5. Return `OutboundReceipt` carrying the published event id and the new
    ///    canonical `message_id`.
    ///
    /// The legacy inbox path is NOT touched here — local delivery is the
    /// caller's responsibility (rpc_send_message keeps route_mention_into_with_id).
    /// Sign a [`SendIntent`] and return the pre-signed Nostr event WITHOUT
    /// submitting it to the relay.  Callers that need to perform local inbox
    /// delivery and `mark_mention_seen` between signing and publishing (to
    /// prevent the relay echo from racing ahead) call this first, act on the
    /// `event.id`, then call [`publish_intent`].
    pub async fn sign_intent(
        &self,
        intent: &SendIntent,
        agent_keys: &nostr_sdk::Keys,
    ) -> Result<nostr_sdk::Event> {
        use nostr_sdk::Tag;
        let root_native_key: Option<String> = intent.thread_id.as_deref().and_then(|tid| {
            self.with_store(|s| s.thread_root_native_key(tid, FABRIC, &self.provider_instance))
        });
        let mention = intent.to_mention();
        let mut builder = self.wire.encode(&DomainEvent::Mention(mention))?;
        if let Some(ref root_key) = root_native_key {
            match Tag::parse(["e", root_key, "", "root"]) {
                Ok(tag) => { builder = builder.tags([tag]); }
                Err(e) => {
                    if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                        eprintln!("[provider] could not build root e-tag: {e:#}");
                    }
                }
            }
        }
        self.transport.sign(builder, agent_keys).await
    }

    /// Publish a pre-signed event and write canonical dual-write rows.
    /// Counterpart to [`sign_intent`].  On relay error, propagates immediately
    /// (no canonical row written).
    pub async fn publish_intent(
        &self,
        intent: SendIntent,
        signed: nostr_sdk::Event,
    ) -> Result<OutboundReceipt> {
        let eid_hex = signed.id.to_hex();
        self.transport.publish_event(&signed).await?;
        let now = now_secs();
        let pi = self.provider_instance.clone();
        let (message_id, thread_id) = self.with_store(|s| -> Result<(String, String)> {
            let project_id =
                s.ensure_project_origin(FABRIC, &pi, &intent.project, &intent.project, now)?;
            let thread_id = if let Some(tid) = intent.thread_id.as_deref() {
                tid.to_string()
            } else {
                s.ensure_thread_origin(&project_id, FABRIC, &pi, &eid_hex, now)?
            };
            let message_id = s.record_message(
                &thread_id,
                &intent.from.pubkey,
                &intent.body,
                now,
                "outbound",
                "published",
                Some(&eid_hex),
            )?;
            s.add_message_recipient(&message_id, &intent.to_pubkey, None)?;
            Ok((message_id, thread_id))
        })?;
        Ok(OutboundReceipt {
            native_event_id: eid_hex,
            message_id,
            sync_state: "published".into(),
            thread_id,
        })
    }

    /// Convenience wrapper: sign + publish in one call.  Equivalent to
    /// `sign_intent` followed immediately by `publish_intent` with no
    /// caller action between the two.  Used by callers that have no local
    /// inbox delivery to perform (e.g. `rpc_inbox_reply`).
    pub async fn send(
        &self,
        intent: SendIntent,
        agent_keys: &nostr_sdk::Keys,
    ) -> Result<OutboundReceipt> {
        let signed = self.sign_intent(&intent, agent_keys).await?;
        self.publish_intent(intent, signed).await
    }

    // ── refresh_project_list ──────────────────────────────────────────────────

    /// Fetch all kind:39000 events from the relay, parse `d` + `about`, and
    /// upsert into `project_meta` (and canonical `projects` via backfill).
    ///
    /// This is the EXACT logic relocated verbatim from `rpc_project_list` in
    /// `daemon/server.rs`. The function is best-effort; callers use `.ok()`.
    pub async fn refresh_project_list(&self) -> Result<()> {
        use nostr_sdk::prelude::{Filter, Kind};
        let filter = Filter::new().kind(Kind::from(39000u16)).limit(200);
        let events = self
            .transport
            .fetch(filter, Duration::from_secs(5))
            .await
            .unwrap_or_default();
        let now = now_secs();
        let pi = self.provider_instance.clone();
        for ev in &events {
            let Some(slug) = crate::fabric::nip29::nostr_tag(ev, "d") else {
                continue;
            };
            let slug = slug.to_string();
            let about = crate::fabric::nip29::nostr_tag(ev, "about")
                .unwrap_or("")
                .to_string();
            self.with_store(|s| {
                // Materialization refresh: update legacy project_meta cache.
                s.upsert_project_meta(&slug, &about, now).ok();
                // Canonical origin row (idempotent).
                s.ensure_project_origin(FABRIC, &pi, &slug, &slug, now).ok();
            });
        }
        Ok(())
    }

    // ── catch_up_mentions ─────────────────────────────────────────────────────

    /// Fetch kind:1 events p-tagged to `rec.agent_pubkey` from the relay and
    /// materialize each through `crate::fabric::materialize`. Returns the number
    /// of events that triggered a mention wake.
    ///
    /// IMPORTANT: uses a single-element `hosted = [rec.agent_pubkey]` slice —
    /// intentionally NOT the daemon's full hosted set — so the Mention guard
    /// (`m.to_pubkey == me`) works exactly (relay filter already restricts).
    /// The caller triggers any live delivery surfaces if the count > 0.
    pub async fn catch_up_mentions(&self, rec: &SessionRecord, owners: &[String]) -> Result<usize> {
        use nostr_sdk::prelude::{Filter, Kind, PublicKey};
        let me = rec.agent_pubkey.clone();
        let pk = PublicKey::from_hex(&me)?;
        let filter = Filter::new().kind(Kind::from(1u16)).pubkey(pk).limit(50);
        let mut wake_count = 0usize;
        if let Ok(events) = self.transport.fetch(filter, Duration::from_secs(3)).await {
            let hosted = vec![me.clone()];
            let now = now_secs();
            let pi = self.provider_instance.clone();
            for ev in events {
                let env = RawEnvelope::Nostr(ev);
                let outcome = self
                    .with_store(|s| crate::fabric::materialize(&env, &hosted, owners, now, &pi, s));
                // NOTE: do NOT send outcome.tail here — fetch is startup catchup only;
                // historical mentions must not be replayed onto the tail channel.
                if outcome.wake_mentions {
                    wake_count += 1;
                }
            }
        }
        Ok(wake_count)
    }

    // ── materialize ───────────────────────────────────────────────────────────

    /// Decode one raw envelope and apply all store side-effects.
    /// Delegates to `crate::fabric::materialize`.
    ///
    /// NOTE: the `store` arg is passed IN by the daemon (already locked via
    /// `state.with_store`) — this method must NOT lock `self.store` again.
    pub fn materialize(
        &self,
        env: &RawEnvelope,
        hosted: &[String],
        owners: &[String],
        now: u64,
        store: &Store,
    ) -> MaterializationOutcome {
        crate::fabric::materialize(env, hosted, owners, now, &self.provider_instance, store)
    }

    // ── private helpers ───────────────────────────────────────────────────────

    fn with_store<R>(&self, f: impl FnOnce(&Store) -> R) -> R {
        let g = self.store.lock().expect("store mutex poisoned");
        f(&g)
    }
}

// ── module-level helpers ──────────────────────────────────────────────────────

/// Derive a stable `provider_instance` string from the relay URL set.
///
/// Sorts + deduplicates the relay URLs, joins them with `|`, hashes with
/// `DefaultHasher` (fixed seed = 0 at the point the hasher is reset), and
/// formats the result as 16 hex digits.  The value is deterministic for the
/// same relay set across daemon restarts on the same machine.
fn derive_provider_instance(relays: &[String]) -> String {
    let mut sorted: Vec<&str> = relays.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    sorted.dedup();
    let joined = sorted.join("|");
    let mut h = DefaultHasher::new();
    joined.hash(&mut h);
    format!("{:016x}", h.finish())
}

// ── unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::AgentRef;
    use crate::fabric::kind1::wire::Kind1WireCodec;
    use nostr_sdk::prelude::{Keys, Tag};

    /// Verify that `builder.tags([root_e_tag])` APPENDS rather than replaces:
    /// the resulting signed event must carry BOTH the original routing tags
    /// (p-tag, h-tag) AND the new root e-tag — all three present.
    #[test]
    fn reply_encoding_preserves_original_tags_and_adds_root_e_tag() {
        let wire = Kind1WireCodec;
        let sender_keys = Keys::generate();
        let recipient_pk = Keys::generate().public_key().to_hex();
        // Use a fixed 64-char hex root key (valid EventId format for Tag::parse).
        let root_hex = "b".repeat(64);

        let mention = crate::domain::Mention {
            from: AgentRef::new(sender_keys.public_key().to_hex(), "sender".to_string()),
            to_pubkey: recipient_pk.clone(),
            project: "myproj".into(),
            body: "hello reply".into(),
            meta: crate::domain::MentionMeta::default(),
        };

        let builder = wire
            .encode(&crate::domain::DomainEvent::Mention(mention))
            .expect("encode succeeded");

        // Append the root e-tag (simulating what provider.send does for replies).
        let root_tag = Tag::parse(["e", &root_hex, "", "root"]).expect("valid root tag");
        let builder = builder.tags([root_tag]);

        // Sign to materialise the final event.
        let event = builder
            .sign_with_keys(&sender_keys)
            .expect("signing succeeded");

        // Collect tag names present on the event.
        let tag_names: Vec<&str> = event
            .tags
            .iter()
            .filter_map(|t| t.as_slice().first().map(String::as_str))
            .collect();

        assert!(
            tag_names.contains(&"p"),
            "p-tag (recipient) must be present; tags: {:?}",
            tag_names
        );
        assert!(
            tag_names.contains(&"h"),
            "h-tag (project/group) must be present; tags: {:?}",
            tag_names
        );
        assert!(
            tag_names.contains(&"e"),
            "e-tag (root) must be present; tags: {:?}",
            tag_names
        );

        // Verify root e-tag has marker "root".
        let root_tag_found = event.tags.iter().any(|t| {
            let s = t.as_slice();
            s.first().map(String::as_str) == Some("e")
                && s.get(3).map(String::as_str) == Some("root")
        });
        assert!(
            root_tag_found,
            "e-tag must carry the 'root' marker; tags: {:?}",
            event.tags
        );
    }
}
