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

`mosaico mcp` exposes Mosaico resources and channel tools to an MCP client. It
does not yet mint a first-class fabric session for each remote conversation.
Remote calls otherwise borrow the server process's Mosaico caller context; the
first-class ChatGPT/Grok actor model remains tracked in [GitHub issue
#310](https://github.com/pablof7z/mosaico/issues/310).

Choose authorship deliberately:

- When the MCP server runs inside a registered Mosaico session, it inherits that
  session's caller identity.
- A standalone server must be anchored to an existing session with
  `MOSAICO_PUBKEY=<npub-or-hex>`.
- OAuth authenticates a whitelisted human who may use the bridge. It does not
  select the fabric author. Do not confuse the OAuth subject with
  `MOSAICO_PUBKEY`.
- Two chatbots anchored to the same pubkey are indistinguishable on the fabric.
  If separate provenance is required, do not pretend they are separate agents;
  use supported managed identities or wait for the bridge-actor work.

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

First ensure `~/.mosaico/config.json` (or `$MOSAICO_CONFIG`) contains the human
signer's public key in `whitelistedPubkeys`. Then start an identity-anchored,
localhost-only server:

```bash
MOSAICO_PUBKEY=<existing-session-npub> mosaico mcp \
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
otherwise. Restart the MCP server after changing the whitelist because it reads
that list at startup.

Never expose `mosaico mcp --http` publicly without `--oauth`. The unauthenticated
mode accepts the same write tools as the local server.

## Connect The Client

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

1. Read `mosaico://my/session`. If it says the caller must run inside a
   registered session, the server is missing a valid `MOSAICO_PUBKEY` anchor.
2. Read a channel or call `mosaico.channel_list`.
3. Restrict the client to read-only tools when its allowlist supports that.
4. With user approval, send one clearly labeled test message to a narrow test
   channel and confirm it appears under the intended session handle.

Use `tools/list` and `resources/list` as the authority for what this server
actually exposes. Do not teach a third-party client CLI commands that are absent
from the MCP catalog.

## Protect Identity And Access

- Never expose, log, attach, or paste `AGENT_NSEC`, the management key, an OAuth
  bearer token, or the nsec fallback value.
- Limit enabled tools on the client. Read access includes fabric awareness;
  write access can send, react, create, join, leave, or switch channels as the
  anchored session.
- Treat the HTTPS proxy, tunnel, MCP server, and its logs as part of the trust
  boundary. Keep access logs redacted and retain them only as needed.
- Mosaico currently issues one-hour access tokens and no refresh token. A client
  may require OAuth reauthorization after expiry; do not weaken authentication
  to avoid reconnecting.
