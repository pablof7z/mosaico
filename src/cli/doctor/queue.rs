//! What this daemon still owes, read from NMP's durable publish queue.
//!
//! The ONE account of outstanding writes. Mosaico used to run a process-local
//! receipt observer beside this, over a bounded observation window; two answers
//! to one question meant a daemon restarted with parked writes could get a
//! clean bill from the half that had forgotten them. This half is durable: a
//! write parked since a previous boot — no signer attached, refused at
//! acceptance — is still visible here.

use super::{Check, CheckStatus};

pub(super) fn inspect(value: &serde_json::Value, checks: &mut Vec<Check>) {
    if let Some(reason) = value
        .get("unreadable")
        .and_then(serde_json::Value::as_str)
        .filter(|reason| !reason.is_empty())
    {
        checks.push(
            Check::new(
                "write.queue",
                CheckStatus::Error,
                format!("NMP's publish queue could not be read: {reason}"),
            )
            .repair("inspect the daemon log for a store fault, then restart the daemon"),
        );
        return;
    }
    let entries = value["entries"].as_u64().unwrap_or_default();
    let outstanding = value["outstanding"].as_u64().unwrap_or_default();
    let stuck_total = value["stuck_total"].as_u64().unwrap_or_default();
    if stuck_total == 0 {
        checks.push(Check::new(
            "write.queue",
            CheckStatus::Ok,
            format!("{outstanding} write(s) in flight, {entries} retained, none stuck"),
        ));
        return;
    }
    checks.push(
        Check::new(
            "write.queue",
            CheckStatus::Warning,
            format!("{stuck_total} write(s) will not progress on their own: {}", named(value)),
        )
        // Deliberately not offered as an automatic repair. Removing an entry
        // discards an obligation someone asked for, so the call is a person's.
        .repair("act on the reason named for each write, or clear the entry deliberately once it is not wanted"),
    );
}

/// The individual writes, as NMP accounts for them.
fn named(value: &serde_json::Value) -> String {
    let Some(stuck) = value["stuck"].as_array() else {
        return "no detail recorded".into();
    };
    let named = stuck
        .iter()
        .map(|write| {
            let id = write["event_id"].as_str().unwrap_or("unknown-event");
            let reason = write["reason"].as_str().unwrap_or("no reason recorded");
            format!("{} ({reason})", crate::util::short_id(id))
        })
        .collect::<Vec<_>>()
        .join("; ");
    let hidden = value["stuck_total"]
        .as_u64()
        .unwrap_or_default()
        .saturating_sub(stuck.len() as u64);
    if hidden == 0 {
        named
    } else {
        format!("{named}; and {hidden} more")
    }
}

#[cfg(test)]
#[path = "queue/tests.rs"]
mod tests;
