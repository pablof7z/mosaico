use crate::who_snapshot::WhoRow;
use owo_colors::OwoColorize;

pub(super) fn row_host_label(row: &WhoRow) -> String {
    let host = row.host.trim();
    let host = if row.remote {
        format!("{host}, remote")
    } else {
        host.to_string()
    };
    rel_cwd_bracket(&row.rel_cwd)
        .map(|dir| format!("{host} [{dir}]"))
        .unwrap_or(host)
}

pub(super) fn row_title_label(row: &WhoRow) -> String {
    if row.status.trim().is_empty() {
        "—".to_string()
    } else {
        row.status.trim().to_string()
    }
}

pub(super) fn row_state_label(row: &WhoRow) -> String {
    if row.dormant {
        return last_active_label(row.age_secs);
    }
    let mut status = if row.active {
        let activity = row.activity.trim();
        if activity.is_empty() {
            "working".to_string()
        } else {
            activity.to_string()
        }
    } else {
        "idle".to_string()
    };
    if !row.fresh {
        status.push_str(" (stale)");
    }
    status
}

/// The `[dir]` to show for a row's `rel_cwd`: `None` when empty or the project
/// root (`.`), so the project root renders without a bracket (§8e).
pub(super) fn rel_cwd_bracket(rel_cwd: &str) -> Option<&str> {
    let r = rel_cwd.trim();
    if r.is_empty() || r == "." {
        None
    } else {
        Some(r)
    }
}

/// Terminal status label: dims the live activity and idle marker so the
/// persistent title stays prominent.
pub(super) fn status_colored(row: &WhoRow) -> String {
    if row.dormant {
        let title = row.status.trim();
        let activity = last_active_label(row.age_secs);
        return if title.is_empty() {
            activity.dimmed().to_string()
        } else {
            format!("{} {}", title, format!("— {activity}").dimmed())
        };
    }
    let t = row.status.trim();
    let a = row.activity.trim();
    match (t.is_empty(), row.active) {
        (true, true) if !a.is_empty() => a.dimmed().to_string(),
        (true, true) => "working".dimmed().to_string(),
        (true, false) => "idle".dimmed().to_string(),
        (false, true) if !a.is_empty() => format!("{} {}", t, format!("— {a}").dimmed()),
        (false, true) => t.to_string(),
        (false, false) => format!("{} {}", t, "· idle".dimmed()),
    }
}

fn last_active_label(age_secs: Option<u64>) -> String {
    match age_secs {
        Some(secs) => format!("last active {} ago", compact_age(secs)),
        None => "last active recently".to_string(),
    }
}

fn compact_age(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}
