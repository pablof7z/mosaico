# Third-Party Chatbots Through MCP

Read this reference only when the user asks to connect ChatGPT, Grok, or another
MCP client to Mosaico.

## Contents

- [Know The Current Boundary](#know-the-current-boundary)
- [Choose A Transport](#choose-a-transport)
- [Expose Remote HTTP Safely](#expose-remote-http-safely)
- [Connect The Client](#connect-the-client)
- [Verify Before Writing](#verify-before-writing)
- [Protect Identity And Access](#protect-identity-and-access)

## Know The Current Boundary

`mosaico mcp` exposes Mosaico resources and channel tools to MCP clients. The
agent skill is also packaged for clients that cannot load a local skill install:
read-only resource `mosaico://skill` (and `mosaico://skill/{name}`), plus the
`mosaico.skill` tool (prefer the tool when the client does not support resources).
MCP clients can block without polling through `mosaico.wait`, or set
`wait_seconds` on `mosaico.channel_send` to await a reply correlated to the sent
message.
Remote HTTP calls do not borrow the server process's caller context:

- ChatGPT gets a first-class `mcp-openai` session and pubkey per stable remote
  conversation. Calls in the same conversation reuse it; different
  conversations do not.
- Grok currently exposes no stable conversation identifier, so all Grok calls
  use one first-class `mcp-grok` session until that changes.
- OAuth authenticates the whitelisted human allowed to use the bridge. Mosaico
  derives a keyed, non-raw actor correlation from the authenticated subject and
  client conversation metadata; raw OpenAI subject, organization, and session
  values are not persisted or logged.
- ChatGPT writes fail closed when no stable conversation identifier is present.
  An explicit public `session` selector remains an operator override.

Local stdio is different: it runs in the caller's local process context and may
inherit or explicitly select an existing session.

## Choose A Transport

Use stdio for a trusted MCP client running on the same machine. A typical client
entry is:

```json
{
  "mcpServers": {
    "mosaico": {
      "command": "mosaico",
      "args": ["mcp"],
      "env": {"MOSAICO_PUBKEY": "<existing-session-npub>"}
    }
  }
}
```

Treat that shape as illustrative; use the client's native configuration format.
Omit the explicit environment only when the client process already inherits a
registered Mosaico session.

Hosted chatbots cannot launch local stdio servers. They need Mosaico's remote
Streamable HTTP endpoint over public HTTPS.

## Expose Remote HTTP Safely

Two operator decisions live in `~/.mosaico/config.json` (or `$MOSAICO_CONFIG`)
and both are read live, on every request:

- `whitelistedPubkeys` — the human signers allowed to log in and to keep
  holding a token.
- `mcpRedirectOrigins` — the origins a hosted MCP client may register a
  callback under, for example `["https://chatgpt.com"]`. A client that runs
  beside Mosaico and calls back to loopback needs no entry here; a hosted
  connector does, and its registration is refused without one.

Set both, then start a standalone, localhost-only server:

```bash
mosaico mcp \
  --http \
  --host 127.0.0.1 \
  --port 8765 \
  --path /mcp \
  --oauth \
  --public-url https://mosaico.example.com
```

`--public-url` is the HTTPS origin, without `/mcp`. Put a TLS reverse proxy or
secure tunnel in front of the local listener and forward the whole origin, not
only `/mcp`: OAuth also uses `/.well-known/*` and `/oauth/*`. Keep the Mosaico
listener bound to localhost unless the network design explicitly requires
otherwise. Removing a pubkey from `whitelistedPubkeys` takes effect on that
operator's next request — no restart, and never by rotating `mosaicoPrivateKey`,
which is the backend's NIP-29 management identity and would destroy every group
membership it owns.

Never expose `mosaico mcp --http` publicly without `--oauth`. The unauthenticated
mode accepts the same write tools as the local server.

## Connect The Client

Every client below identifies itself by registering at `/oauth/register`, which
records the exact redirect URIs that `client_id` may receive an authorization
code at. If registration is refused, the callback origin is not in
`mcpRedirectOrigins` — add it there rather than working around the refusal. The
Mosaico authorization page shows the registered client and the exact callback
before the signer is asked to approve; if either is unfamiliar, do not sign.

- **ChatGPT:** use the current Settings > Apps / developer-mode flow to create a
  custom MCP app, enter `https://mosaico.example.com/mcp`, select OAuth, complete
  the Mosaico authorization page, and scan tools. Availability and UI vary by
  plan; use the [official ChatGPT MCP app
  guide](https://help.openai.com/en/articles/12584461-developer-mode-and-full-mcp-connectors-in-chatgpt-beta)
  as the client-side authority.
- **Grok:** open `grok.com/connectors`, create a Custom connector, enter the same
  public MCP URL, and complete supported authentication. The [official Grok
  connector guide](https://docs.x.ai/grok/connectors) is the client-side
  authority. For API-driven Grok, follow its [Remote MCP tool
  guide](https://docs.x.ai/developers/tools/remote-mcp).
- **Other clients:** select Streamable HTTP, use the public `/mcp` URL, and use
  OAuth discovery with PKCE when supported. If a client only accepts a static
  bearer token, Mosaico currently has no durable static-token flow; do not work
  around that by disabling public authentication.

Prefer NIP-07 on the Mosaico authorization page. Never ask the user to paste an
`nsec` into chat. The fallback nsec field is for the user to enter directly into
their own trusted HTTPS authorization page.

## Verify Before Writing

Check the public routing without credentials:

```bash
curl -fsS https://mosaico.example.com/
curl -fsS https://mosaico.example.com/.well-known/oauth-protected-resource
```

Then use the MCP client to scan tools and resources. Verify in this order:

1. Call `mosaico.skill` (or read `mosaico://skill`) and confirm the skill entry
   markdown returns without a daemon dependency.
2. Read `mosaico://my/session`. A remote ChatGPT call must report an
   `mcp-openai` session, never the MCP server process session and never a demand
   to run inside a registered agent session.
3. Repeat the read in the same conversation and confirm the exact session
   pubkey is stable. A separate conversation must receive a different pubkey.
4. Read a channel or call `mosaico.channel_list`.
5. Restrict the client to read-only tools when its allowlist supports that.
6. With user approval, send one clearly labeled test message to a narrow test
   channel and confirm it appears under the derived MCP actor handle.

Use `tools/list` and `resources/list` as the authority for what this server
actually exposes. Do not teach a third-party client CLI commands that are absent
from the MCP catalog.

## Protect Identity And Access

- Never expose, log, attach, or paste `AGENT_NSEC`, the management key, an OAuth
  bearer token, or the nsec fallback value.
- Limit enabled tools on the client. Read access includes fabric awareness;
  write access can send, react, create, join, or leave channels as the
  resolved MCP actor session.
- Treat the HTTPS proxy, tunnel, MCP server, and its logs as part of the trust
  boundary. Keep access logs redacted and retain them only as needed.
- Mosaico issues one-hour access tokens and no refresh token. A client may
  require OAuth reauthorization after expiry; do not weaken authentication to
  avoid reconnecting.
- To revoke an operator, remove their pubkey from `whitelistedPubkeys`. Every
  token verification re-reads that list, so the withdrawal lands on their next
  request. Never revoke by rotating `mosaicoPrivateKey`.
