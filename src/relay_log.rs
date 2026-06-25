//! Persistent log of every outgoing relay event and every rejection.
//!
//! Appends timestamped one-liners to `~/.tenex-edge/relay.log` (always) and
//! echoes them to stderr. Fail-open: if the file cannot be opened the calls
//! are no-ops beyond the stderr echo.
//!
//! ```text
//! 2026-06-25 14:32:05.123  [→relay] kind:9000  put-user  h=my-project  p=abc12345  role=admin
//! 2026-06-25 14:32:05.456  [→relay] kind:9007  create-group  h=session-xyz  parent=my-project
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
            let dir = crate::config::tenex_dir();
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
    let ts = crate::util::format_local_datetime_ms(crate::util::now_millis());
    let full = format!("{ts}  {line}");
    eprintln!("{full}");
    if let Some(mu) = log_file() {
        if let Ok(mut f) = mu.lock() {
            let _ = writeln!(f, "{full}");
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

    let line = match k {
        // kind:0 is published to the indexer relay (purplepag.es) for profile
        // discovery — not meaningful relay traffic for the configured relay log.
        0 => return,
        9 => format!("[→relay] kind:{k:<5}  orchestration  h={h}  →{ps}"),
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
            format!("[→relay] kind:{k:<5}  put-user  h={h}  p={ps}  role={role}")
        }
        9001 => format!("[→relay] kind:{k:<5}  remove-user  h={h}  p={ps}"),
        9002 => {
            let name = tv("name");
            let parent = tv("parent");
            if parent != "-" {
                format!("[→relay] kind:{k:<5}  edit-metadata  h={h}  name={name:?}  parent={parent}")
            } else {
                format!("[→relay] kind:{k:<5}  edit-metadata  h={h}  name={name:?}")
            }
        }
        9007 => {
            let parent = tv("parent");
            if parent != "-" {
                format!("[→relay] kind:{k:<5}  create-group  h={h}  parent={parent}")
            } else {
                format!("[→relay] kind:{k:<5}  create-group  h={h}")
            }
        }
        30023 => format!(
            "[→relay] kind:{k:<5}  proposal  h={h}  title={:?}",
            tv("title")
        ),
        30315 => format!(
            "[→relay] kind:{k:<5}  status  h={h}  {}  title={:?}",
            tv("status"),
            tv("title")
        ),
        _ => format!(
            "[→relay] kind:{k:<5}  h={h}  author={}",
            &event.pubkey.to_hex()[..8]
        ),
    };

    log_entry(&line);
}

/// Log a rejection before [`Transport`] returns the error to its caller.
pub(crate) fn log_relay_rejection(reason: &str) {
    log_entry(&format!("[relay✗] rejected: {reason}"));
}
