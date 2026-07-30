use crate::state::{RelayEvent, Session, Store};

const HISTORY_SCAN_LIMIT: u32 = 1_000;
const MAX_CLUSTER_SPAN_SECS: u64 = 60 * 60;
const MAX_EMPTY_GAP_SECS: u64 = 20 * 60;
const FRESH_CLUSTER_SECS: u64 = 10 * 60;

pub(crate) fn prejoin_notices(
    store: &Store,
    session: &Session,
    joined: &[(String, u64)],
    now: u64,
) -> anyhow::Result<Vec<String>> {
    let mut notices = Vec::new();
    for (channel, _) in joined {
        if let Some(notice) = prejoin_notice(store, session, channel, now)? {
            notices.push(notice);
        }
    }
    Ok(notices)
}

pub(crate) fn prejoin_notice(
    store: &Store,
    session: &Session,
    channel: &str,
    now: u64,
) -> anyhow::Result<Option<String>> {
    let events = store.prejoin_chat_for_session(&session.pubkey, channel, HISTORY_SCAN_LIMIT)?;
    let Some(cluster) = newest_cluster(&events) else {
        return Ok(None);
    };
    let channel_ref = crate::channel_ref::full_channel_ref(store, channel);
    if channel_ref.is_empty() {
        return Ok(None);
    }
    Ok(Some(render_notice(store, &channel_ref, cluster, now)))
}

fn newest_cluster(events: &[RelayEvent]) -> Option<&[RelayEvent]> {
    let newest = events.first()?.created_at;
    let mut previous = newest;
    let mut len = 0;
    for event in events {
        if newest.saturating_sub(event.created_at) > MAX_CLUSTER_SPAN_SECS
            || previous.saturating_sub(event.created_at) > MAX_EMPTY_GAP_SECS
        {
            break;
        }
        previous = event.created_at;
        len += 1;
    }
    Some(&events[..len])
}

fn render_notice(store: &Store, channel_ref: &str, events: &[RelayEvent], now: u64) -> String {
    let newest = events.first().map(|event| event.created_at).unwrap_or(now);
    let oldest = events
        .last()
        .map(|event| event.created_at)
        .unwrap_or(newest);
    let span = newest.saturating_sub(oldest).max(60);
    let age = now.saturating_sub(newest);
    let authors = render_authors(store, events);
    let count = events.len();
    let message_word = if count == 1 { "message" } else { "messages" };
    let summary = if age <= FRESH_CLUSTER_SECS {
        format!(
            "{count} {message_word} in the recent pre-join activity in {channel_ref} \
             over {}, from {authors}.",
            relative_duration(span)
        )
    } else {
        format!(
            "{count} {message_word} in a prior {} activity cluster in {channel_ref}, \
             last active {} ago, from {authors}.",
            relative_duration(span),
            relative_duration(age)
        )
    };
    format!(
        "{summary} To inspect earlier conversation, read \
         `~/.agents/skills/mosaico/references/coordination-guide.md` and use an explicit channel read."
    )
}

fn render_authors(store: &Store, events: &[RelayEvent]) -> String {
    let mut authors = Vec::new();
    for event in events {
        if authors
            .iter()
            .any(|pubkey: &String| pubkey == &event.pubkey)
        {
            continue;
        }
        authors.push(event.pubkey.clone());
    }
    let labels = authors
        .iter()
        .take(3)
        .map(|pubkey| {
            let label = store
                .resolve_slug_for_pubkey(pubkey)
                .ok()
                .flatten()
                .unwrap_or_else(|| crate::util::pubkey_short(pubkey));
            format!("@{}", label.trim_start_matches('@'))
        })
        .collect::<Vec<_>>();
    if authors.len() > labels.len() {
        format!(
            "{} and {} {}",
            labels.join(", "),
            authors.len() - labels.len(),
            if authors.len() - labels.len() == 1 {
                "other"
            } else {
                "others"
            }
        )
    } else {
        labels.join(", ")
    }
}

fn relative_duration(seconds: u64) -> String {
    if seconds < 60 * 60 {
        format!("{} min", seconds.div_ceil(60).max(1))
    } else if seconds < 24 * 60 * 60 {
        format!("{} hr", seconds.div_ceil(60 * 60))
    } else {
        format!("{} days", seconds.div_ceil(24 * 60 * 60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: &str, at: u64) -> RelayEvent {
        RelayEvent {
            id: id.into(),
            kind: 9,
            pubkey: format!("author-{id}"),
            created_at: at,
            channel_h: "room".into(),
            d_tag: String::new(),
            content: String::new(),
            tags_json: "[]".into(),
        }
    }

    #[test]
    fn newest_cluster_stops_at_twenty_minute_silence() {
        let rows = vec![event("a", 10_000), event("b", 9_700), event("c", 1_000)];
        assert_eq!(newest_cluster(&rows).unwrap().len(), 2);
    }

    #[test]
    fn newest_cluster_reports_recent_burst_instead_of_large_old_burst() {
        let mut rows = (0..10)
            .map(|index| event(&format!("recent-{index}"), 10_000 - index * 30))
            .collect::<Vec<_>>();
        rows.extend((0..45).map(|index| event(&format!("old-{index}"), 1_000 - index * 20)));

        let cluster = newest_cluster(&rows).unwrap();
        assert_eq!(cluster.len(), 10);
        assert_eq!(cluster.first().unwrap().id, "recent-0");
        assert_eq!(cluster.last().unwrap().id, "recent-9");
    }

    #[test]
    fn newest_cluster_is_capped_at_one_hour() {
        let rows = vec![
            event("a", 10_000),
            event("b", 9_000),
            event("c", 8_000),
            event("d", 7_000),
            event("e", 6_000),
        ];
        assert_eq!(newest_cluster(&rows).unwrap().len(), 4);
    }

    #[test]
    fn relative_duration_is_compact() {
        assert_eq!(relative_duration(1), "1 min");
        assert_eq!(relative_duration(3_601), "2 hr");
        assert_eq!(relative_duration(86_401), "2 days");
    }

    #[test]
    fn notice_uses_the_installed_coordination_guide_path() {
        let store = Store::open_memory().unwrap();
        let events = vec![event("a", 10_000)];
        let notice = render_notice(&store, "#room", &events, 10_000);
        assert!(
            notice.contains("`~/.agents/skills/mosaico/references/coordination-guide.md`"),
            "{notice}"
        );
        assert!(!notice.contains("mosaico://"), "{notice}");
    }
}
