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
            if e.path().extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            if let Ok(s) = std::fs::read_to_string(e.path()) {
                if let Ok(k) = serde_json::from_str::<StoredKey>(&s) {
                    out.push(k.public_key);
                }
            }
        }
    }
    out
}

/// All agents in the local keystore with their configured harness command (if
/// any) and display byline. Used by the spawn machinery: command from the agent
/// file takes priority over SPAWN_DEFS.
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
            if let Ok(s) = std::fs::read_to_string(&path) {
                if let Ok(k) = serde_json::from_str::<StoredKey>(&s) {
                    let byline = k.effective_byline();
                    out.push((k.slug, k.command, k.agent, byline));
                }
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
            if let Ok(s) = std::fs::read_to_string(&path) {
                if let Ok(k) = serde_json::from_str::<StoredKey>(&s) {
                    out.push(LocalAgent {
                        slug: k.slug,
                        pubkey: k.public_key,
                        command: k.command,
                    });
                }
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
///                    `"opencode"`.
/// - `anchor`       — native harness session id (claude/codex) or canonical
///                    daemon SessionId (opencode).  Stage 1 is anchor-agnostic;
///                    the caller decides which value to pass.
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

/// Write via a temp file + rename so a crash never leaves a half-written key.
fn atomic_write(path: &Path, body: &str) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_then_reloads_same_key() {
        let dir = tempfile::tempdir().unwrap();
        let a = load_or_create(dir.path(), "coder", 100).unwrap();
        let b = load_or_create(dir.path(), "coder", 200).unwrap();
        assert_eq!(a.pubkey_hex(), b.pubkey_hex());
        assert_eq!(
            a.keys.secret_key().to_secret_hex(),
            b.keys.secret_key().to_secret_hex()
        );
    }

    #[test]
    fn distinct_slugs_get_distinct_keys() {
        let dir = tempfile::tempdir().unwrap();
        let a = load_or_create(dir.path(), "coder", 1).unwrap();
        let b = load_or_create(dir.path(), "reviewer", 1).unwrap();
        assert_ne!(a.pubkey_hex(), b.pubkey_hex());
    }

    #[test]
    fn rejects_bad_slug() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_or_create(dir.path(), "bad slug/with-stuff", 1).is_err());
        assert!(load_or_create(dir.path(), "", 1).is_err());
    }

    #[test]
    fn persists_to_expected_path() {
        let dir = tempfile::tempdir().unwrap();
        load_or_create(dir.path(), "coder", 1).unwrap();
        assert!(dir.path().join("agents").join("coder.json").exists());
    }

    #[test]
    fn add_local_agent_creates_then_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let (a, created) = add_local_agent(dir.path(), "coder", None, 1).unwrap();
        assert!(created, "first add mints a fresh key");
        assert!(dir.path().join("agents").join("coder.json").exists());

        // Second add with no command returns the SAME key, created=false.
        let (b, created2) = add_local_agent(dir.path(), "coder", None, 2).unwrap();
        assert!(!created2, "re-adding an existing slug does not recreate");
        assert_eq!(a.pubkey_hex(), b.pubkey_hex());
    }

    #[test]
    fn add_local_agent_sets_and_overwrites_command() {
        let dir = tempfile::tempdir().unwrap();
        // Create with a command.
        let (a, _) = add_local_agent(
            dir.path(),
            "dev",
            Some(vec![
                "claude".into(),
                "--dangerously-skip-permissions".into(),
            ]),
            1,
        )
        .unwrap();
        assert_eq!(
            a.command.as_deref().unwrap(),
            &["claude", "--dangerously-skip-permissions"]
        );
        // Overwrite the command on the existing agent; key is unchanged.
        let (b, created) =
            add_local_agent(dir.path(), "dev", Some(vec!["codex".into()]), 2).unwrap();
        assert!(!created);
        assert_eq!(a.pubkey_hex(), b.pubkey_hex());
        assert_eq!(b.command.as_deref().unwrap(), &["codex"]);
    }

    #[test]
    fn remove_local_agent_parks_then_reports_missing() {
        let dir = tempfile::tempdir().unwrap();
        load_or_create(dir.path(), "coder", 1).unwrap();
        let live = dir.path().join("agents").join("coder.json");
        assert!(live.exists());

        let parked = remove_local_agent(dir.path(), "coder").unwrap();
        let parked = parked.expect("removing an existing agent returns the parked path");
        assert!(!live.exists(), "live key file is gone");
        assert!(parked.exists(), "key is parked, not unlinked");
        // Parked file is not a `.json`, so it drops out of the listings.
        assert!(list_local_agent_details(dir.path()).is_empty());
        assert!(list_local_pubkeys(dir.path()).is_empty());

        // Removing again is a no-op (None), not an error.
        assert!(remove_local_agent(dir.path(), "coder").unwrap().is_none());
    }

    #[test]
    fn list_local_agent_details_surfaces_pubkey_and_command() {
        let dir = tempfile::tempdir().unwrap();
        let (a, _) = add_local_agent(dir.path(), "coder", None, 1).unwrap();
        add_local_agent(dir.path(), "dev", Some(vec!["codex".into()]), 1).unwrap();
        let rows = list_local_agent_details(dir.path());
        assert_eq!(rows.len(), 2);
        // Sorted by slug: coder, dev.
        assert_eq!(rows[0].slug, "coder");
        assert_eq!(rows[0].pubkey, a.pubkey_hex());
        assert!(rows[0].command.is_none());
        assert_eq!(rows[1].slug, "dev");
        assert_eq!(rows[1].command.as_deref().unwrap(), &["codex"]);
    }

    #[test]
    fn command_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        // Write a file with a command field manually
        std::fs::create_dir_all(dir.path().join("agents")).unwrap();
        std::fs::write(
            dir.path().join("agents/dev.json"),
            r#"{"slug":"dev","secret_key":"0000000000000000000000000000000000000000000000000000000000000001","public_key":"","created_at":1,"command":["claude","--dangerously-skip-permissions"]}"#,
        )
        .unwrap();
        let agents = list_local_agents(dir.path());
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].0, "dev");
        assert_eq!(
            agents[0].1.as_deref().unwrap(),
            &["claude", "--dangerously-skip-permissions"]
        );
        assert!(agents[0].2.is_none());
        assert!(agents[0].3.is_none());
    }

    // -----------------------------------------------------------------------
    // derive_session_keys tests
    // -----------------------------------------------------------------------

    /// Fixed tenex secret used across all derivation tests.
    fn test_tenex_secret() -> SecretKey {
        // 0x01 repeated 32 times — valid, non-trivial, easy to reproduce.
        SecretKey::from_slice(&[0x01u8; 32]).unwrap()
    }

    #[test]
    fn session_key_determinism() {
        // Same inputs → identical keypair on every call (resume property).
        let sk = test_tenex_secret();
        let a = derive_session_keys(&sk, "my-project", "coder", "claude", "sess-abc");
        let b = derive_session_keys(&sk, "my-project", "coder", "claude", "sess-abc");
        assert_eq!(
            a.public_key().to_hex(),
            b.public_key().to_hex(),
            "derive_session_keys must be deterministic"
        );
        assert_eq!(
            a.secret_key().to_secret_hex(),
            b.secret_key().to_secret_hex(),
        );
    }

    #[test]
    fn session_key_different_anchors_differ() {
        // Two different anchors (same project/agent/harness) → different pubkeys.
        let sk = test_tenex_secret();
        let a = derive_session_keys(&sk, "proj", "coder", "claude", "session-1");
        let b = derive_session_keys(&sk, "proj", "coder", "claude", "session-2");
        assert_ne!(
            a.public_key().to_hex(),
            b.public_key().to_hex(),
            "different anchors must yield different session pubkeys"
        );
    }

    #[test]
    fn session_key_different_projects_differ() {
        // Different project_slug → different pubkey (cross-project isolation).
        let sk = test_tenex_secret();
        let a = derive_session_keys(&sk, "project-alpha", "coder", "claude", "anchor-x");
        let b = derive_session_keys(&sk, "project-beta", "coder", "claude", "anchor-x");
        assert_ne!(
            a.public_key().to_hex(),
            b.public_key().to_hex(),
            "different project slugs must yield different session pubkeys"
        );
    }

    #[test]
    fn session_key_different_agent_slugs_differ() {
        // Different agent_slug → different pubkey.
        let sk = test_tenex_secret();
        let a = derive_session_keys(&sk, "proj", "coder", "claude", "anchor");
        let b = derive_session_keys(&sk, "proj", "reviewer", "claude", "anchor");
        assert_ne!(
            a.public_key().to_hex(),
            b.public_key().to_hex(),
            "different agent slugs must yield different session pubkeys"
        );
    }

    #[test]
    fn session_key_field_boundary_non_collision() {
        // ("a", "bc") must differ from ("ab", "c") — NUL-delimiter property.
        // Without proper encoding these can collide if fields are naively concatenated.
        let sk = test_tenex_secret();
        let a = derive_session_keys(&sk, "a", "bc", "claude", "anchor");
        let b = derive_session_keys(&sk, "ab", "c", "claude", "anchor");
        assert_ne!(
            a.public_key().to_hex(),
            b.public_key().to_hex(),
            "field-boundary collision: (project='a', agent='bc') must differ from (project='ab', agent='c')"
        );
    }

    #[test]
    fn session_key_known_answer() {
        // Pinned known-answer: hardcoded inputs → hardcoded pubkey hex.
        // If derivation logic changes, this test catches the regression.
        // Computed by running the first passing test suite; do not change
        // unless the derivation spec itself changes (and bump the salt version).
        let sk = test_tenex_secret();
        let keys = derive_session_keys(&sk, "my-project", "coder", "claude", "sess-abc");
        // Pin: computed from HKDF-SHA256(ikm=[0x01;32], salt="tenex-edge/session-key/v1",
        //      info="my-project\0coder\0claude\0sess-abc\0\x00")
        assert_eq!(
            keys.public_key().to_hex(),
            "9aa6883eee2f1ce43053a1eec2c1c8b1c712cbb3c77ec346d9f091982a50b461",
            "known-answer test: pinned pubkey changed — was the derivation spec modified?"
        );
    }

    // -----------------------------------------------------------------------
    // Issue #2 acceptance-criteria tests (AC1, AC2, AC3).
    // These complement the generic determinism/isolation tests above with
    // assertions framed exactly as the acceptance criteria state them.
    // -----------------------------------------------------------------------

    /// AC1: A resumed harness session MUST derive the SAME session pubkey as the
    /// original run. The session's Nostr wire identity is stable across restarts
    /// as long as the same anchor inputs are used (same operator key + project +
    /// agent + harness kind + harness-native session id).
    #[test]
    fn ac1_resumed_session_derives_same_pubkey() {
        let sk = test_tenex_secret();
        let harness_id = "claude-native-xKz8-resume-test";

        let original = derive_session_keys(&sk, "my-project", "coder", "claude", harness_id);
        let resumed = derive_session_keys(&sk, "my-project", "coder", "claude", harness_id);

        assert_eq!(
            original.public_key().to_hex(),
            resumed.public_key().to_hex(),
            "AC1: a resumed harness session must reproduce the exact same session pubkey"
        );
        assert_eq!(
            original.secret_key().to_secret_hex(),
            resumed.secret_key().to_secret_hex(),
            "AC1: and the exact same secret key (full keypair is deterministic)"
        );
    }

    /// AC2: Two DIFFERENT harness sessions for the SAME durable agent must
    /// produce different session pubkeys, ensuring each session has its own
    /// routable wire identity.
    #[test]
    fn ac2_two_sessions_same_agent_different_pubkeys() {
        let sk = test_tenex_secret();

        let session_a = derive_session_keys(&sk, "proj", "coder", "claude", "native-id-aaaa");
        let session_b = derive_session_keys(&sk, "proj", "coder", "claude", "native-id-bbbb");

        assert_ne!(
            session_a.public_key().to_hex(),
            session_b.public_key().to_hex(),
            "AC2: two distinct harness sessions for the same agent must have different pubkeys"
        );
    }

    /// AC3: Two projects must NOT share a derived session identity for the same
    /// harness id, preventing cross-project routing leakage.
    #[test]
    fn ac3_same_harness_id_different_projects_isolate() {
        let sk = test_tenex_secret();
        let anchor = "same-harness-id-across-projects";

        let proj_alpha = derive_session_keys(&sk, "project-alpha", "coder", "claude", anchor);
        let proj_beta = derive_session_keys(&sk, "project-beta", "coder", "claude", anchor);

        assert_ne!(
            proj_alpha.public_key().to_hex(),
            proj_beta.public_key().to_hex(),
            "AC3: same harness id must yield different session pubkeys in different projects"
        );
    }

    #[test]
    fn byline_reads_field_alias_and_falls_back_to_agent_description() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("agents")).unwrap();
        // Explicit `byline`.
        std::fs::write(
            dir.path().join("agents/a.json"),
            r#"{"slug":"a","secret_key":"0000000000000000000000000000000000000000000000000000000000000001","public_key":"","created_at":1,"byline":"front-line triage"}"#,
        )
        .unwrap();
        // `useCriteria` alias.
        std::fs::write(
            dir.path().join("agents/b.json"),
            r#"{"slug":"b","secret_key":"0000000000000000000000000000000000000000000000000000000000000002","public_key":"","created_at":1,"useCriteria":"use for deep research"}"#,
        )
        .unwrap();
        // Falls back to the inline agent definition's `description`.
        std::fs::write(
            dir.path().join("agents/c.json"),
            r#"{"slug":"c","secret_key":"0000000000000000000000000000000000000000000000000000000000000003","public_key":"","created_at":1,"agent":{"description":"writes social posts"}}"#,
        )
        .unwrap();
        // No byline anywhere.
        std::fs::write(
            dir.path().join("agents/d.json"),
            r#"{"slug":"d","secret_key":"0000000000000000000000000000000000000000000000000000000000000004","public_key":"","created_at":1}"#,
        )
        .unwrap();

        let agents = list_local_agents(dir.path());
        let byline = |slug: &str| {
            agents
                .iter()
                .find(|a| a.0 == slug)
                .and_then(|a| a.3.clone())
        };
        assert_eq!(byline("a").as_deref(), Some("front-line triage"));
        assert_eq!(byline("b").as_deref(), Some("use for deep research"));
        assert_eq!(byline("c").as_deref(), Some("writes social posts"));
        assert_eq!(byline("d"), None);
    }
}
