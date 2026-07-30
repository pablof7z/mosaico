# Message search

Use `mosaico channel search` to find messages already materialized in the
daemon's local database. Search never queries or backfills from the relay, so a
missing result means only that no matching message is present in this local
cache.

## Scope

Omitting `--channel` and passing `--channel /` are equivalent: both search
every channel currently represented in the local database. A narrower channel
path includes that channel and every descendant:

```bash
mosaico channel search --channel /nmp/research
```

There is no workspace filter. A workspace root is an ordinary channel path, so
`--channel /nmp` searches that root and its descendants.

Do not add a local permission check before searching. NIP-29 relay policy owns
admission and authorization. Once messages have been accepted and materialized
by the current backend, channel and workspace paths are query scopes rather
than separate local visibility boundaries.

## Filters

Search supports these repeatable filters:

- `--from <IDENTITY>` matches an author by handle, npub, hex pubkey, or another
  identity reference accepted by Mosaico.
- `--to <IDENTITY>` matches an explicit message recipient.
- `--contains <TEXT>` performs a case-insensitive literal body match.
- `--channel <PATH>` searches the named channel and its descendants.

It also supports `--since <TIME>`, `--until <TIME>`, `--limit <COUNT>`, and
`--cursor <CURSOR>`. Times may be Unix timestamps or relative durations such
as `30m`, `2h`, or `7d`.

Repeated values within one filter are OR alternatives. Different filter kinds
combine with AND. For example:

```bash
mosaico channel search \
  --from @Pablo --from @reviewer \
  --contains commit \
  --channel /nmp/research \
  --since 7d
```

This finds locally cached messages authored by Pablo **or** reviewer whose body
contains `commit`, within `/nmp/research` or any descendant, during the last
seven days.

## Output and pagination

Results are selected newest-first and grouped by channel in agent-native XML:

```xml
<mosaico>
  <channel ref="/nmp/research">
    <message from="@Pablo" for="@reviewer" id="4e91c0" age="4m">landed the commit</message>
  </channel>
  <channel ref="/nmp/research/design">
    <message from="@reviewer" id="7bc421" time="1785348600">approved</message>
  </channel>
  <next cursor="opaque-cursor" />
</mosaico>
```

Messages at most one hour old use compact relative `age` values without
“ago”. Older messages use the absolute Unix timestamp in `time`. The shared
message renderer owns sender and recipient labels, short IDs, escaping,
truncation, and recovery hints.

When `<next>` is present, pass its opaque value unchanged to `--cursor` alone.
The cursor contains the normalized query and page position:

```bash
mosaico channel search --cursor 'opaque-cursor'
```

Do not inspect or construct a cursor, and do not combine it with filters or a
new limit. To recover the complete body for a returned message, use its short
ID:

```bash
mosaico channel read --id 4e91c0
```

The MCP tool `mosaico.channel_search` accepts the same filters as typed fields.
Its text content is the same XML. Its `structuredContent` mirrors the grouped
channels, messages, and optional pagination cursor without changing the search
semantics.
