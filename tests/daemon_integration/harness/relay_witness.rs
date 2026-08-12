use anyhow::{bail, Context, Result};
use mosaico::domain::{AgentRef, ChatMessage, DomainEvent};
use mosaico::fabric::nip29::wire::Nip29WireCodec;
use nostr::{Event, EventBuilder, Keys};
use std::collections::BTreeSet;
use std::fs::File;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const NAK_QUERY_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) async fn publish_addressed_chat(
    relay: &str,
    operator_nsec: &str,
    channel: &str,
    body: &str,
    target_pubkey: &str,
) -> String {
    let keys = Keys::parse(operator_nsec).expect("operator keys");
    let client = crate::nmp_client::NmpRelayClient::connect(keys.clone(), relay)
        .await
        .expect("connect independent NMP relay client");
    let chat = ChatMessage {
        from: AgentRef::new(keys.public_key().to_hex(), ""),
        channel: channel.to_string(),
        body: body.to_string(),
        mentioned_pubkeys: vec![target_pubkey.to_string()],
        attachments: Vec::new(),
    };
    let builder = Nip29WireCodec
        .encode_event(&DomainEvent::ChatMessage(chat))
        .expect("encode addressed kind:9");
    let event = sign_builder_into_group(channel, builder, &keys)
        .sign_with_keys(&keys)
        .expect("sign addressed kind:9");
    let outcome = client
        .send_event(&event)
        .await
        .expect("publish addressed kind:9");
    assert!(
        !outcome.success.is_empty(),
        "operator kind:9 was rejected: {:?}",
        outcome.failed
    );
    client.disconnect().await;
    event.id.to_hex()
}

pub(crate) fn sign_builder_into_group(
    group: &str,
    builder: EventBuilder,
    keys: &Keys,
) -> EventBuilder {
    let unsigned = builder.build(keys.public_key());
    let contextualized = nmp_nip29::contextualize(
        &std::collections::BTreeSet::from([group.to_string()]),
        nmp::EventBuilder {
            kind: unsigned.kind,
            tags: unsigned.tags.into_iter().collect(),
            content: unsigned.content,
            created_at: Some(unsigned.created_at),
        },
    )
    .expect("a fixture draft carries no context row of its own");
    EventBuilder::new(contextualized.kind, contextualized.content)
        .tags(contextualized.tags)
        .custom_created_at(contextualized.created_at.expect("stated above"))
        .allow_self_tagging()
}

/// Compare the complete set of current relay snapshots containing `pubkey`.
///
/// The query is external to the daemon and bounded. Every returned event must
/// be a valid signed kind:39002 snapshot with the requested `p` tag.
pub(crate) fn wait_for_exact_relay_groups(
    relay: &str,
    pubkey: &str,
    expected: &BTreeSet<String>,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    loop {
        let last = match relay_groups_for_member(relay, pubkey) {
            Ok(actual) if &actual == expected => return,
            Ok(actual) => format!("actual groups: {actual:?}"),
            Err(error) => format!("query error: {error:#}"),
        };
        if Instant::now() >= deadline {
            panic!("relay membership for {pubkey} did not converge to {expected:?}; {last}");
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// Wait until the relay's current signed kind:39000 names the exact parent.
///
/// Session rooms are deliberately unnamed and therefore have no public path;
/// their relay-signed parent row is the independent topology witness.
pub(crate) fn wait_for_relay_group_parent(
    relay: &str,
    group: &str,
    expected_parent: &str,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    loop {
        let last = match relay_parent_for_group(relay, group) {
            Ok(Some(parent)) if parent == expected_parent => return,
            Ok(Some(parent)) => format!("actual parent: {parent:?}"),
            Ok(None) => "no current kind:39000".to_string(),
            Err(error) => format!("query error: {error:#}"),
        };
        if Instant::now() >= deadline {
            panic!("relay parent for {group} did not converge to {expected_parent:?}; {last}");
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn relay_parent_for_group(relay: &str, group: &str) -> Result<Option<String>> {
    let stdout = run_nak_req_bounded(relay, &["-k", "39000", "-d", group])?;
    let mut parents = BTreeSet::new();
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let event: Event = serde_json::from_str(line).context("parse relay kind:39000 event")?;
        event
            .verify()
            .context("verify relay kind:39000 signature")?;
        if event.kind.as_u16() != 39_000 {
            bail!(
                "relay query returned unexpected kind {}",
                event.kind.as_u16()
            );
        }
        let value: serde_json::Value =
            serde_json::from_str(line).context("parse relay metadata tags")?;
        let tags = value["tags"]
            .as_array()
            .context("kind:39000 tags are not an array")?;
        let event_group = tag_value(tags, "d").context("kind:39000 has no group identifier")?;
        if event_group == group {
            parents.insert(tag_value(tags, "parent").unwrap_or_default());
        }
    }
    if parents.len() > 1 {
        bail!("relay returned conflicting current parents for {group}: {parents:?}");
    }
    Ok(parents.into_iter().next())
}

fn tag_value(tags: &[serde_json::Value], name: &str) -> Option<String> {
    tags.iter().find_map(|tag| {
        let parts = tag.as_array()?;
        (parts.first()?.as_str()? == name)
            .then(|| parts.get(1)?.as_str().map(str::to_string))
            .flatten()
    })
}

fn relay_groups_for_member(relay: &str, pubkey: &str) -> Result<BTreeSet<String>> {
    let stdout = run_nak_req_bounded(relay, &["-k", "39002", "-p", pubkey])?;
    let mut groups = BTreeSet::new();
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let event: Event = serde_json::from_str(line).context("parse relay kind:39002 event")?;
        event
            .verify()
            .context("verify relay kind:39002 signature")?;
        if event.kind.as_u16() != 39_002 {
            bail!(
                "relay query returned unexpected kind {}",
                event.kind.as_u16()
            );
        }
        let value: serde_json::Value =
            serde_json::from_str(line).context("parse relay event tags")?;
        let tags = value["tags"]
            .as_array()
            .context("kind:39002 tags are not an array")?;
        let includes_member = tags.iter().any(|tag| {
            tag.as_array().is_some_and(|parts| {
                parts.first().and_then(serde_json::Value::as_str) == Some("p")
                    && parts.get(1).and_then(serde_json::Value::as_str) == Some(pubkey)
            })
        });
        if !includes_member {
            bail!("relay query returned a snapshot without requested member {pubkey}");
        }
        let group = tags
            .iter()
            .find_map(|tag| {
                let parts = tag.as_array()?;
                (parts.first()?.as_str()? == "d")
                    .then(|| parts.get(1)?.as_str().map(str::to_string))
                    .flatten()
            })
            .context("kind:39002 snapshot has no group identifier")?;
        groups.insert(group);
    }
    Ok(groups)
}

fn run_nak_req_bounded(relay: &str, filters: &[&str]) -> Result<String> {
    let scratch = tempfile::tempdir().context("create nak query scratch")?;
    let stdout_path = scratch.path().join("stdout.log");
    let stderr_path = scratch.path().join("stderr.log");
    let stdout = File::create(&stdout_path).context("create nak stdout")?;
    let stderr = File::create(&stderr_path).context("create nak stderr")?;
    let mut args = vec!["req"];
    args.extend_from_slice(filters);
    args.push(relay);
    let mut child = Command::new(crate::common::nak_bin())
        .args(args)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .context("spawn independent nak membership query")?;
    let deadline = Instant::now() + NAK_QUERY_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait().context("poll nak membership query")? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let stdout = std::fs::read_to_string(&stdout_path).unwrap_or_default();
            let stderr = std::fs::read_to_string(&stderr_path).unwrap_or_default();
            bail!(
                "nak membership query exceeded {NAK_QUERY_TIMEOUT:?}; stdout={stdout:?}; stderr={stderr:?}"
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let stdout = std::fs::read_to_string(&stdout_path).context("read nak stdout")?;
    let stderr = std::fs::read_to_string(&stderr_path).context("read nak stderr")?;
    if !status.success() {
        bail!("nak membership query failed with {status}; stdout={stdout:?}; stderr={stderr:?}");
    }
    Ok(stdout)
}
