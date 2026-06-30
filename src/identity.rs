//! Agent keystore (M1 §4).
//!
//! `--agent <slug>` resolves to a durable Nostr keypair, generated on first use
//! and persisted under `<edge_home>/agents/<slug>.json`. Identity is
//! `(agent, machine)`: the same slug on another machine is a different key.
//!
//! NOTE: agent keypairs live under `<edge_home>/agents/<slug>.json`, which
//! defaults to `~/.tenex-edge/agents/`. `edge_home()` defaults to `~/.tenex-edge`.

use anyhow::{bail, Context, Result};
use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
struct StoredKey {
    slug: String,
    secret_key: String, // hex
    public_key: String, // hex
    created_at: u64,
    /// Harness command to use when spawning a new tmux session for this agent.
    /// E.g. `["claude", "--dangerously-skip-permissions"]`.
    /// When absent, the spawn logic falls back to the built-in SPAWN_DEFS table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    command: Option<Vec<String>>,
    /// Inline agent definition forwarded to the harness at spawn time.
    /// For Claude: becomes `--agents '{"<slug>": <def>}' --agent <slug>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent: Option<serde_json::Value>,
    /// One-line "when to use this agent" note, surfaced in `who`'s agent table.
    /// Read from `byline` or its alias `useCriteria`.
    #[serde(
        default,
        alias = "useCriteria",
        skip_serializing_if = "Option::is_none"
    )]
    byline: Option<String>,
}

impl StoredKey {
    /// The byline to display for this agent: the explicit `byline`/`useCriteria`
    /// field, falling back to the inline agent definition's `description`.
    fn effective_byline(&self) -> Option<String> {
        self.byline
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                self.agent
                    .as_ref()
                    .and_then(|a| a.get("description"))
                    .and_then(|d| d.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            })
    }
}

/// A resolved agent identity: its slug, signing keys, and optional harness command.
#[derive(Debug, Clone)]
pub struct AgentIdentity {
    pub slug: String,
    pub keys: Keys,
    /// Harness command from the agent file, if present.
    pub command: Option<Vec<String>>,
}

impl AgentIdentity {
    pub fn pubkey_hex(&self) -> String {
        self.keys.public_key().to_hex()
    }
}

fn agents_dir(edge_home: &Path) -> PathBuf {
    edge_home.join("agents")
}

fn key_path(edge_home: &Path, slug: &str) -> PathBuf {
    agents_dir(edge_home).join(format!("{slug}.json"))
}

fn validate_slug(slug: &str) -> Result<()> {
    if slug.is_empty()
        || !slug
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        bail!("invalid agent slug {slug:?}: use [A-Za-z0-9._-]");
    }
    Ok(())
}

/// Load the agent's keypair, generating + persisting it on first use.
pub fn load_or_create(edge_home: &Path, slug: &str, now: u64) -> Result<AgentIdentity> {
    validate_slug(slug)?;
    let path = key_path(edge_home, slug);
    if path.exists() {
        let s = std::fs::read_to_string(&path)
            .with_context(|| format!("reading key {}", path.display()))?;
        let stored: StoredKey =
            serde_json::from_str(&s).with_context(|| format!("parsing key {}", path.display()))?;
        let keys = Keys::parse(&stored.secret_key)
            .with_context(|| format!("parsing secret key for {slug}"))?;
        tracing::debug!(slug, pubkey = %&stored.public_key[..8], "agent key loaded");
        return Ok(AgentIdentity {
            slug: slug.to_string(),
            keys,
            command: stored.command,
        });
    }

    let keys = Keys::generate();
    let stored = StoredKey {
        slug: slug.to_string(),
        secret_key: keys.secret_key().to_secret_hex(),
        public_key: keys.public_key().to_hex(),
        created_at: now,
        command: None,
        agent: None,
        byline: None,
    };
    std::fs::create_dir_all(agents_dir(edge_home))
        .with_context(|| format!("creating {}", agents_dir(edge_home).display()))?;
    let body = serde_json::to_string_pretty(&stored)?;
    atomic_write(&path, &body)?;
    tracing::info!(slug, pubkey = %&stored.public_key[..8], path = %path.display(), "agent key created");
    Ok(AgentIdentity {
        slug: slug.to_string(),
        keys,
        command: None,
    })
}

/// Every agent in the local keystore (their hex pubkeys). Your own fleet trusts
/// itself automatically, so agents on one device see each other without the
/// operator having to pre-whitelist keys that are generated on first use.
pub fn list_local_pubkeys(edge_home: &Path) -> Vec<String> {
    let dir = agents_dir(edge_home);
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            // This list is the self-trust whitelist: a key silently skipped here
            // drops an agent's own fleet member out of the trusted set, so a
            // read/parse failure is loud at error! before we skip it.
            match std::fs::read_to_string(&path) {
                Ok(s) => match serde_json::from_str::<StoredKey>(&s) {
                    Ok(k) => out.push(k.public_key),
                    Err(e) => tracing::error!(
                        path = %path.display(),
                        error = %e,
                        "list_local_pubkeys: skipping corrupt keystore file — agent dropped from self-trust whitelist"
                    ),
                },
                Err(e) => tracing::error!(
                    path = %path.display(),
                    error = %e,
                    "list_local_pubkeys: skipping unreadable keystore file — agent dropped from self-trust whitelist"
                ),
            }
        }
    }
    out
}

/// All agents in the local keystore with their configured harness command (if
/// any) and display byline. Used by the spawn machinery: command from the agent
/// file takes priority over SPAWN_DEFS.
#[allow(clippy::type_complexity)]
pub fn list_local_agents(
    edge_home: &Path,
) -> Vec<(
    String,
    Option<Vec<String>>,
    Option<serde_json::Value>,
    Option<String>,
)> {
    let dir = agents_dir(edge_home);
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(s) => match serde_json::from_str::<StoredKey>(&s) {
                    Ok(k) => {
                        let byline = k.effective_byline();
                        out.push((k.slug, k.command, k.agent, byline));
                    }
                    Err(e) => tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "list_local_agents: skipping corrupt keystore file"
                    ),
                },
                Err(e) => tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "list_local_agents: skipping unreadable keystore file"
                ),
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// A local agent as listed by `tenex-edge agent list`: its slug, hex pubkey, and
/// optional harness launch command. Distinct from `list_local_agents` (which the
/// spawn path uses) in that it also surfaces the pubkey for the operator.
#[derive(Debug, Clone)]
pub struct LocalAgent {
    pub slug: String,
    pub pubkey: String,
    pub command: Option<Vec<String>>,
}

/// The invitable roster as `(slug, byline, created_at)`, sorted by slug. The
/// `created_at` lets the awareness delta surface only AGENTS THAT BECAME
/// available since a session's last turn (decision D — roster shown on change,
/// not every turn).
pub fn list_invitable_agents(edge_home: &Path) -> Vec<(String, Option<String>, u64)> {
    let dir = agents_dir(edge_home);
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(s) => match serde_json::from_str::<StoredKey>(&s) {
                    Ok(k) => {
                        let byline = k.effective_byline();
                        out.push((k.slug, byline, k.created_at));
                    }
                    Err(e) => tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "list_invitable_agents: skipping corrupt keystore file"
                    ),
                },
                Err(e) => tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "list_invitable_agents: skipping unreadable keystore file"
                ),
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Every agent in the local keystore, with slug + pubkey + command, sorted by slug.
pub fn list_local_agent_details(edge_home: &Path) -> Vec<LocalAgent> {
    let dir = agents_dir(edge_home);
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(s) => match serde_json::from_str::<StoredKey>(&s) {
                    Ok(k) => out.push(LocalAgent {
                        slug: k.slug,
                        pubkey: k.public_key,
                        command: k.command,
                    }),
                    Err(e) => tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "list_local_agent_details: skipping corrupt keystore file"
                    ),
                },
                Err(e) => tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "list_local_agent_details: skipping unreadable keystore file"
                ),
            }
        }
    }
    out.sort_by(|a, b| a.slug.cmp(&b.slug));
    out
}

/// Add a local agent: mint + persist a keypair if the slug is new. When `command`
/// is `Some`, set (or overwrite) the harness launch command — so this doubles as
/// "set the command for an existing agent". Returns the resolved identity and
/// whether the keypair was newly created (`true`) or already existed (`false`).
pub fn add_local_agent(
    edge_home: &Path,
    slug: &str,
    command: Option<Vec<String>>,
    now: u64,
) -> Result<(AgentIdentity, bool)> {
    validate_slug(slug)?;
    let path = key_path(edge_home, slug);
    if path.exists() {
        let s = std::fs::read_to_string(&path)
            .with_context(|| format!("reading key {}", path.display()))?;
        let mut stored: StoredKey =
            serde_json::from_str(&s).with_context(|| format!("parsing key {}", path.display()))?;
        let keys = Keys::parse(&stored.secret_key)
            .with_context(|| format!("parsing secret key for {slug}"))?;
        if command.is_some() {
            stored.command = command;
            let body = serde_json::to_string_pretty(&stored)?;
            atomic_write(&path, &body)?;
        }
        return Ok((
            AgentIdentity {
                slug: slug.to_string(),
                keys,
                command: stored.command,
            },
            false,
        ));
    }

    let keys = Keys::generate();
    let stored = StoredKey {
        slug: slug.to_string(),
        secret_key: keys.secret_key().to_secret_hex(),
        public_key: keys.public_key().to_hex(),
        created_at: now,
        command: command.clone(),
        agent: None,
        byline: None,
    };
    std::fs::create_dir_all(agents_dir(edge_home))
        .with_context(|| format!("creating {}", agents_dir(edge_home).display()))?;
    let body = serde_json::to_string_pretty(&stored)?;
    atomic_write(&path, &body)?;
    Ok((
        AgentIdentity {
            slug: slug.to_string(),
            keys,
            command,
        },
        true,
    ))
}

/// Remove a local agent by soft-deleting its keystore file: the private key is
/// renamed to `<slug>.json.removed` rather than unlinked, so a mistaken removal
/// is recoverable with a single `mv` (a freshly minted key would otherwise be a
/// *different* identity, losing the agent's pubkey forever). Returns the path the
/// key was parked at, or `None` if no such agent existed.
pub fn remove_local_agent(edge_home: &Path, slug: &str) -> Result<Option<PathBuf>> {
    validate_slug(slug)?;
    let path = key_path(edge_home, slug);
    if !path.exists() {
        return Ok(None);
    }
    let parked = path.with_extension("json.removed");
    std::fs::rename(&path, &parked)
        .with_context(|| format!("parking {} -> {}", path.display(), parked.display()))?;
    Ok(Some(parked))
}

// ---------------------------------------------------------------------------
// Session-key derivation (Stage 1 / Issue #2)
// ---------------------------------------------------------------------------

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// HKDF-SHA256: Extract then Expand to produce exactly 32 bytes of keying
/// material.  We only ever need one output block (L = 32 = HashLen), so the
/// Expand step is a single HMAC invocation.
///
/// - Extract:  PRK = HMAC-SHA256(salt, IKM)
/// - Expand:   OKM = HMAC-SHA256(PRK, info || 0x01)   [T(1), L ≤ 32]
fn hkdf_sha256_32(ikm: &[u8], salt: &[u8], info: &[u8]) -> [u8; 32] {
    // Extract
    let mut mac = HmacSha256::new_from_slice(salt).expect("HMAC accepts any key length");
    mac.update(ikm);
    let prk: [u8; 32] = mac.finalize().into_bytes().into();

    // Expand – single block (counter byte 0x01 is the HKDF block index)
    let mut mac = HmacSha256::new_from_slice(&prk).expect("PRK is always 32 bytes");
    mac.update(info);
    mac.update(&[0x01u8]);
    mac.finalize().into_bytes().into()
}

/// Deterministically derive a per-session keypair.  Same inputs → same key,
/// so a resumed harness session reproduces its pubkey.  The session key is a
/// live routable actor only; the tenex key stays NIP-29 admin.
///
/// # Info encoding
///
/// `info` is built as NUL-delimited (0x00) fields:
///
/// ```text
/// project_slug '\0' agent_slug '\0' harness_kind '\0' anchor '\0' counter
/// ```
///
/// NUL delimiters prevent cross-field collisions: `("a", "bc")` produces
/// `a\0bc\0…` while `("ab", "c")` produces `ab\0c\0…` — these differ at the
/// first NUL position so the two are always distinct.
///
/// The final byte is a rejection-sampling counter (starts at 0x00).  On the
/// negligible chance that the raw HKDF output is an invalid secp256k1 scalar,
/// the counter is incremented and HKDF-Expand is re-run against the same PRK
/// with the mutated info, avoiding any ad-hoc fallback.
///
/// # Inputs
///
/// - `tenex_secret` — the configured operator private key (IKM, 32 bytes).
/// - `project_slug` — logical project identifier.
/// - `agent_slug`   — agent role within the project.
/// - `harness_kind` — harness type string, e.g. `"claude"`, `"codex"`,
///   `"opencode"`.
/// - `anchor`       — native harness session id (claude/codex) or canonical
///   daemon SessionId (opencode).  Stage 1 is anchor-agnostic;
///   the caller decides which value to pass.
pub fn derive_session_keys(
    tenex_secret: &SecretKey,
    project_slug: &str,
    agent_slug: &str,
    harness_kind: &str,
    anchor: &str,
) -> Keys {
    const SALT: &[u8] = b"tenex-edge/session-key/v1";
    let ikm = tenex_secret.as_secret_bytes();

    // Build the NUL-delimited info buffer.  The last byte is reserved for the
    // rejection-sampling counter; we mutate it in place on retry.
    let mut info: Vec<u8> = Vec::with_capacity(
        project_slug.len()
            + 1
            + agent_slug.len()
            + 1
            + harness_kind.len()
            + 1
            + anchor.len()
            + 1
            + 1,
    );
    info.extend_from_slice(project_slug.as_bytes());
    info.push(0x00);
    info.extend_from_slice(agent_slug.as_bytes());
    info.push(0x00);
    info.extend_from_slice(harness_kind.as_bytes());
    info.push(0x00);
    info.extend_from_slice(anchor.as_bytes());
    info.push(0x00);
    info.push(0x00); // counter starts at 0

    let counter_idx = info.len() - 1;

    loop {
        let okm = hkdf_sha256_32(ikm, SALT, &info);
        match SecretKey::from_slice(&okm) {
            Ok(sk) => return Keys::new(sk),
            Err(_) => {
                // The probability that a random 32-byte value is not a valid
                // secp256k1 scalar is ~2^-128.  Guard the counter anyway so
                // the loop is provably finite.
                let counter = info[counter_idx];
                assert!(
                    counter < 255,
                    "derive_session_keys: exhausted rejection counter (astronomically improbable)"
                );
                info[counter_idx] = counter + 1;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Durable ordinal identities (issue #47)
// ---------------------------------------------------------------------------

/// Display label for an agent's Nth concurrent identity. Ordinal 0 is the base
/// agent itself (`smith`); higher ordinals append the number (`smith1`,
/// `smith2`). This is the addressable identity peers see.
pub fn agent_ordinal_label(agent_slug: &str, ordinal: u32) -> String {
    if ordinal == 0 {
        agent_slug.to_string()
    } else {
        format!("{agent_slug}{ordinal}")
    }
}

/// Deterministically derive the keypair for an agent's Nth concurrent identity.
///
/// Replaces `derive_session_keys` for live signer selection (issue #47). The old
/// model derived a fresh key per harness *session* (anchored to the native
/// session id), so the set of live pubkeys churned with every session and leaked
/// relay subscriptions. Ordinal keys are DURABLE: a `(agent, ordinal)` pair maps
/// to one stable pubkey that is REUSED across rooms, so the subscription
/// `#p`-set is bounded by (agents × concurrency high-water mark), not sessions.
///
/// - Ordinal `0` is exactly the base file-backed agent key — no derivation.
/// - Ordinal `N > 0` is HKDF-SHA256 of the base agent secret. The base secret is
///   already unique per `(agent, machine)`, so it alone keys the family; the
///   base pubkey is folded into `info` only to make the derivation explicit and
///   self-describing. There is deliberately NO project/room/session input — the
///   same `smithN` must be the same pubkey everywhere.
///
/// # Info encoding
///
/// ```text
/// base_pubkey_hex '\0' ordinal_be(4) '\0' counter
/// ```
///
/// The trailing byte is a rejection-sampling counter (starts at 0x00), mutated
/// in place on the astronomically improbable chance the raw HKDF output is not a
/// valid secp256k1 scalar — same guard as `derive_session_keys`.
pub fn derive_agent_ordinal_keys(base: &Keys, ordinal: u32) -> Keys {
    if ordinal == 0 {
        return base.clone();
    }
    const SALT: &[u8] = b"tenex-edge/agent-ordinal-key/v1";
    let ikm = base.secret_key().as_secret_bytes();
    let base_pub = base.public_key().to_hex();
    let ord_be = ordinal.to_be_bytes();

    let mut info: Vec<u8> = Vec::with_capacity(base_pub.len() + 1 + 4 + 1 + 1);
    info.extend_from_slice(base_pub.as_bytes());
    info.push(0x00);
    info.extend_from_slice(&ord_be);
    info.push(0x00);
    info.push(0x00); // counter starts at 0

    let counter_idx = info.len() - 1;
    loop {
        let okm = hkdf_sha256_32(ikm, SALT, &info);
        match SecretKey::from_slice(&okm) {
            Ok(sk) => return Keys::new(sk),
            Err(_) => {
                let counter = info[counter_idx];
                assert!(
                    counter < 255,
                    "derive_agent_ordinal_keys: exhausted rejection counter (astronomically improbable)"
                );
                info[counter_idx] = counter + 1;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Agent-instance identity (issue #98)
// ---------------------------------------------------------------------------

/// The single authoritative identity of one running agent instance.
///
/// Issue #98: identity policy — which key signs, which slug displays, which
/// pubkey is used for membership/routing — used to be reconstructed at ~6 edge
/// sites (status publish, chat publish, profile publish, `who`,
/// statusline, mention routing), each free to disagree. The recurring bug:
/// signing as the `claude1` pubkey while labelling the event `claude`, or
/// checking membership for the base pubkey while the instance signs as an
/// ordinal.
///
/// This value is created ONCE at session birth and threaded through (or
/// projected from the `identities` table for read-side callers). Every base-vs-
/// ordinal decision — display label, selected pubkey, signing key — lives in the
/// methods here and NOWHERE else. After session creation, callsites consume an
/// `AgentInstance`; they never re-derive `keys_for_session(..).unwrap_or(base)`
/// or `identity_for_session(..).unwrap_or(rec.agent_slug)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInstance {
    /// The durable agent role (`claude`). Same for every ordinal of the agent.
    pub base_slug: String,
    /// The ordinal-0 (durable, file-backed) pubkey of the agent.
    pub base_pubkey: String,
    /// 0 for the durable instance, N for the Nth concurrent same-agent instance.
    pub ordinal: u32,
    /// This instance's selected pubkey: equals `base_pubkey` iff `ordinal == 0`,
    /// else the deterministically-derived ordinal pubkey.
    pub pubkey: String,
}

impl AgentInstance {
    /// The base (ordinal-0) instance of an agent — the safety fallback for a
    /// session with no derived identity bound yet. `pubkey` IS `base_pubkey`.
    pub fn base(base_slug: String, base_pubkey: String) -> Self {
        Self {
            base_slug,
            base_pubkey: base_pubkey.clone(),
            ordinal: 0,
            pubkey: base_pubkey,
        }
    }

    /// Build from a persisted `identities`-row projection: the row already
    /// carries the base slug/pubkey, ordinal, and selected pubkey.
    pub fn from_parts(
        base_slug: String,
        base_pubkey: String,
        ordinal: u32,
        pubkey: String,
    ) -> Self {
        Self {
            base_slug,
            base_pubkey,
            ordinal,
            pubkey,
        }
    }

    /// The display label peers see: `claude` for ordinal 0, `claude1`, `claude2`
    /// for higher ordinals. The ONE definition of an instance's display name.
    pub fn display_slug(&self) -> String {
        agent_ordinal_label(&self.base_slug, self.ordinal)
    }

    /// The wire identity for every event this instance authors (kind:0, kind:9,
    /// kind:30315): its selected pubkey paired with its display label — NEVER the
    /// base slug paired with an ordinal pubkey.
    pub fn agent_ref(&self) -> crate::domain::AgentRef {
        crate::domain::AgentRef::new(self.pubkey.clone(), self.display_slug())
    }

    /// The keys this instance signs with: the durable base keys for ordinal 0,
    /// the deterministically-derived ordinal keys otherwise. `base_keys` is the
    /// agent's file-backed keypair (ordinal 0). The ONE place the base-vs-ordinal
    /// signing decision is made.
    pub fn signing_keys(&self, base_keys: &Keys) -> Keys {
        derive_agent_ordinal_keys(base_keys, self.ordinal)
    }
}

// ---------------------------------------------------------------------------

/// Write via a temp file + rename so a crash never leaves a half-written key.
fn atomic_write(path: &Path, body: &str) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests;
