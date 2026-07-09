//! Persistent log of every outgoing relay event and every rejection.
//!
//! Appends timestamped one-liners to `~/.tenex-edge/relay.log` (always) and
//! echoes them to stderr. Fail-open: if the file cannot be opened the calls
//! are no-ops beyond the stderr echo.
//!
//! ```text
//! 2026-06-25 14:32:05.123  [→relay] kind:9000  put-user  h=my-channel  p=abc12345  role=admin
//! 2026-06-25 14:32:05.456  [→relay] kind:9007  create-group  h=session-xyz  parent=my-channel
//! 2026-06-25 14:32:05.789  [relay✗] rejected: blocked: unknown group member
//! ```

use nostr_sdk::prelude::Event;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{Mutex, OnceLock};

static LOG_FILE: OnceLock<Option<Mutex<File>>> = OnceLock::new();

fn log_file() -> Option<&'static Mutex<File>> {
    LOG_FILE
        .get_or_init(|| {
            let dir = crate::config::edge_home();
            let _ = std::fs::create_dir_all(&dir);
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("relay.log"))
                .ok()
                .map(Mutex::new)
        })
        .as_ref()
}

fn log_entry(line: &str) {
    // Write to relay.log for forensics, and route through tracing at debug
    // so it only appears in daemon stdout when RUST_LOG=tenex_edge=debug.
    tracing::debug!("{line}");
    if let Some(mu) = log_file() {
        let ts = crate::util::format_local_datetime_ms(crate::util::now_millis());
        if let Ok(mut f) = mu.lock() {
            let _ = writeln!(f, "{ts}  {line}");
        }
    }
}

// ── public API ────────────────────────────────────────────────────────────────

/// Log a one-line summary of `event` before it hits the wire.
pub(crate) fn log_outgoing_event(event: &Event) {
    let k = event.kind.as_u16();

    let tv = |name: &str| -> &str {
        event
            .tags
            .iter()
            .find_map(|t| {
                let s = t.as_slice();
                (s.first().map(String::as_str) == Some(name))
                    .then(|| s.get(1).map(String::as_str))
                    .flatten()
            })
            .unwrap_or("-")
    };

    let p_tags: Vec<String> = event
        .tags
        .iter()
        .filter_map(|t| {
            let s = t.as_slice();
            (s.first().map(String::as_str) == Some("p"))
                .then(|| s.get(1).map(|pk| pk.chars().take(8).collect()))
                .flatten()
        })
        .collect();
    let ps = p_tags.join(",");
    let h = tv("h");

    let id = &event.id.to_hex()[..12];

    let line = match k {
        // kind:0 is published to the indexer relay (purplepag.es) for profile
        // discovery — not meaningful relay traffic for the configured relay log.
        0 => return,
        9 => format!("[→relay] kind:{k:<5}  id={id}  orchestration  h={h}  →{ps}"),
        9000 => {
            let role = event
                .tags
                .iter()
                .find_map(|t| {
                    let s = t.as_slice();
                    (s.first().map(String::as_str) == Some("p") && s.len() > 2)
                        .then(|| s[2].as_str())
                })
                .unwrap_or("member");
            format!("[→relay] kind:{k:<5}  id={id}  put-user  h={h}  p={ps}  role={role}")
        }
        9001 => format!("[→relay] kind:{k:<5}  id={id}  remove-user  h={h}  p={ps}"),
        9002 => {
            let name = tv("name");
            let parent = tv("parent");
            if parent != "-" {
                format!("[→relay] kind:{k:<5}  id={id}  edit-metadata  h={h}  name={name:?}  parent={parent}")
            } else {
                format!("[→relay] kind:{k:<5}  id={id}  edit-metadata  h={h}  name={name:?}")
            }
        }
        9007 => {
            let parent = tv("parent");
            if parent != "-" {
                format!("[→relay] kind:{k:<5}  id={id}  create-group  h={h}  parent={parent}")
            } else {
                format!("[→relay] kind:{k:<5}  id={id}  create-group  h={h}")
            }
        }
        30023 => format!(
            "[→relay] kind:{k:<5}  id={id}  proposal  h={h}  title={:?}",
            tv("title")
        ),
        30315 => format!(
            "[→relay] kind:{k:<5}  id={id}  status  h={h}  {}  title={:?}",
            tv("status"),
            tv("title")
        ),
        _ => format!(
            "[→relay] kind:{k:<5}  id={id}  h={h}  author={}",
            &event.pubkey.to_hex()[..8]
        ),
    };

    log_entry(&line);
}

/// Log a rejection before [`Transport`] returns the error to its caller.
/// If `event` is provided, the kind, id prefix, and h-tag are included.
pub(crate) fn log_relay_rejection(reason: &str, event: Option<&Event>) {
    let event_info = event
        .map(|e| {
            let h = e
                .tags
                .iter()
                .find_map(|t| {
                    let s = t.as_slice();
                    (s.first().map(String::as_str) == Some("h"))
                        .then(|| s.get(1).cloned())
                        .flatten()
                })
                .unwrap_or_default();
            format!(
                "kind:{}  id={}  h={h}  ",
                e.kind.as_u16(),
                &e.id.to_hex()[..12]
            )
        })
        .unwrap_or_default();
    log_entry(&format!("[relay✗] rejected: {event_info}{reason}"));
}
