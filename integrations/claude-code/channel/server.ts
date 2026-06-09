#!/usr/bin/env bun
//
// tenex-edge ↔ Claude Code channel server.
//
// A Claude Code "channel" is an MCP server (stdio) that PUSHES events into a
// running Claude Code session as <channel source="tenex-edge" ...> tags, and
// (optionally) exposes a reply tool so Claude can answer back.
//
// What this server does:
//   * INBOUND  — keeps `tenex-edge wait-for-mention` armed in a re-spawn loop.
//                Every mention it prints becomes one `notifications/claude/channel`
//                event. This REPLACES the old hack where the agent manually re-ran
//                `wait-for-mention` in the background after each message.
//   * OUTBOUND — a `reply` MCP tool that shells `tenex-edge send-message` so Claude
//                can answer the sender it was mentioned by.
//
// Gating: mentions handed to us by `wait-for-mention` are ALREADY owner-scoped /
// ACL-allowlisted by the tenex-edge substrate (it only delivers mentions from
// authorized agents). So this channel inherits upstream gating — we do NOT
// reimplement an allowlist here. See README.md.
//
// IMPORTANT: stdout is the MCP JSON-RPC wire. NOTHING may write to stdout except
// the MCP SDK. All diagnostics go to stderr via `log()`.
//
import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import {
  ListToolsRequestSchema,
  CallToolRequestSchema,
} from "@modelcontextprotocol/sdk/types.js";

// ── config ────────────────────────────────────────────────────────────────

// Bare `tenex-edge` is usually NOT on PATH in the env Claude Code spawns us with,
// so resolve via TENEX_EDGE_BIN, then fall back to ~/.local/bin.
const BIN =
  process.env.TENEX_EDGE_BIN ||
  `${process.env.HOME}/.local/bin/tenex-edge`;

// Session id exported by the SessionStart hook (TENEX_EDGE_SESSION). When set we
// pass it through to `wait-for-mention`/`send-message`; when unset the CLI
// resolves the latest live session for the current project from its cwd.
const SESSION = process.env.TENEX_EDGE_SESSION || "";

// On error (e.g. "no active session" before the SessionStart hook has run), wait
// this long before re-arming so we don't tight-spin.
const ERROR_BACKOFF_MS = 3000;

function log(...args: unknown[]) {
  // stderr ONLY — never console.log, which would corrupt the MCP wire.
  console.error("[tenex-edge channel]", ...args);
}

// ── MCP server ──────────────────────────────────────────────────────────────

const mcp = new Server(
  { name: "tenex-edge", version: "0.1.0" },
  {
    capabilities: {
      experimental: { "claude/channel": {} }, // registers the channel listener
      tools: {}, // two-way: exposes the `reply` tool
    },
    instructions:
      'Events tagged <channel source="tenex-edge" ...> are inbound MENTIONS from ' +
      "other agents on the tenex-edge fabric (your own fleet and your operator's, " +
      "across Claude Code / Codex / opencode). The tag attributes carry provenance: " +
      "`sender` (agent slug), `project`, and `reply_to` (the recipient id to answer). " +
      "These mentions are already owner-scoped and ACL-allowlisted by the tenex-edge " +
      "substrate, so they come from authorized agents. To respond, call the `reply` " +
      "tool, passing the tag's `reply_to` value as `recipient` and your answer as " +
      "`message`. You do NOT need to run `tenex-edge wait-for-mention` yourself — this " +
      "channel keeps the mention listener armed for you automatically.",
  }
);

// ── outbound: the reply tool ──────────────────────────────────────────────────

mcp.setRequestHandler(ListToolsRequestSchema, async () => ({
  tools: [
    {
      name: "reply",
      description:
        "Reply to a tenex-edge mention by sending a message back to the sender. " +
        "Pass the `reply_to` attribute from the inbound <channel> tag as `recipient`.",
      inputSchema: {
        type: "object",
        properties: {
          recipient: {
            type: "string",
            description:
              "Who to send to: the `reply_to` value from the inbound tag " +
              "(an agent slug, slug@project, session-id, or hex pubkey).",
          },
          message: {
            type: "string",
            description: "The message body to send.",
          },
        },
        required: ["recipient", "message"],
      },
    },
  ],
}));

mcp.setRequestHandler(CallToolRequestSchema, async (req) => {
  if (req.params.name !== "reply") {
    throw new Error(`unknown tool: ${req.params.name}`);
  }
  const { recipient, message } = req.params.arguments as {
    recipient: string;
    message: string;
  };
  if (!recipient || !message) {
    return {
      isError: true,
      content: [
        { type: "text", text: "reply requires both `recipient` and `message`." },
      ],
    };
  }

  // Array args (never a shell string): mention bodies contain quotes/newlines.
  const args = ["send-message"];
  if (SESSION) args.push("--session", SESSION);
  args.push(recipient, message);

  try {
    const proc = Bun.spawn([BIN, ...args], {
      stdout: "pipe",
      stderr: "pipe",
    });
    const code = await proc.exited;
    const out = (await new Response(proc.stdout).text()).trim();
    const err = (await new Response(proc.stderr).text()).trim();
    if (code !== 0) {
      log("send-message failed", { code, err });
      return {
        isError: true,
        content: [
          {
            type: "text",
            text: `send-message exited ${code}: ${err || out || "(no output)"}`,
          },
        ],
      };
    }
    return {
      content: [{ type: "text", text: `sent to ${recipient}` }],
    };
  } catch (e) {
    log("send-message spawn error", e);
    return {
      isError: true,
      content: [{ type: "text", text: `failed to send: ${String(e)}` }],
    };
  }
});

// ── inbound: parse a wait-for-mention line into a channel event ───────────────

// Lines look like: `[mention from slug@project] body`
// `wait-for-mention` also prints a trailing hint line we must drop:
//   `[tenex-edge] Run `tenex-edge wait-for-mention` ...`
const MENTION_RE = /^\[mention from ([^@\]]+)@([^\]]+)\] ([\s\S]*)$/;

// meta keys must be /[A-Za-z0-9_]+/ — hyphens & other chars are silently dropped
// by Claude Code. Keep keys underscore-only and aligned with `instructions`.
function emitMention(slug: string, project: string, body: string) {
  const recipient = `${slug}@${project}`;
  log("emit mention", { from: recipient, len: body.length });
  // Fire-and-forget; notifications are not acknowledged.
  mcp
    .notification({
      method: "notifications/claude/channel",
      params: {
        content: body,
        meta: {
          sender: slug,
          project: project,
          // `reply_to` is the id Claude passes back to the `reply` tool. Today
          // the only id recoverable from wait-for-mention output is slug@project;
          // send-message accepts that form directly.
          reply_to: recipient,
        },
      },
    })
    .catch((e) => log("notification error", e));
}

// Parse a full wait-for-mention stdout dump (one batch) into mentions and emit
// each. Non-matching lines that aren't the trailing hint are folded into the
// previous mention's body (mentions are usually single-line, but bodies *can*
// contain newlines — don't orphan them).
function parseAndEmit(stdout: string) {
  const lines = stdout.split("\n");
  let cur: { slug: string; project: string; body: string } | null = null;
  const flush = () => {
    if (cur) emitMention(cur.slug, cur.project, cur.body);
    cur = null;
  };
  for (const line of lines) {
    if (line.startsWith("[tenex-edge] ")) {
      // The re-arm hint line — drop it (we ARE the re-arm loop now).
      continue;
    }
    const m = MENTION_RE.exec(line);
    if (m) {
      flush();
      cur = { slug: m[1], project: m[2], body: m[3] };
    } else if (cur && line !== "") {
      // continuation of a multi-line body (skip blank lines, e.g. the trailing
      // newline at end of the wait-for-mention dump).
      cur.body += "\n" + line;
    }
    // else: stray/blank line before any mention — ignore.
  }
  flush();
}

// ── inbound: the re-arm loop ──────────────────────────────────────────────────
//
// TODO(daemon): when `tenex-edge subscribe --json` lands (a streaming source
// that prints one mention per line as it arrives), REPLACE this re-spawn loop
// with a single long-lived subscribe process: read its stdout line-by-line and
// call emitMention() per line. That removes the 300s respawn churn. It's a
// localized change — only this function and the parser need to move from
// "batch on exit" to "line on arrival". Build against today's wait-for-mention.
//
async function rearmLoop() {
  log("re-arm loop started; bin =", BIN, "session =", SESSION || "(cwd-resolved)");
  for (;;) {
    try {
      const args = ["wait-for-mention"];
      if (SESSION) args.push("--session", SESSION);
      // default --timeout 300; on timeout it exits 0 with no mention lines.

      const proc = Bun.spawn([BIN, ...args], {
        // We OWN the child's stdout — it must NOT inherit to our stdout (the MCP
        // wire). Pipe and consume it ourselves.
        stdout: "pipe",
        stderr: "pipe",
      });
      const code = await proc.exited;
      const stdout = await new Response(proc.stdout).text();
      const stderr = (await new Response(proc.stderr).text()).trim();

      if (code === 0) {
        if (stdout.trim()) parseAndEmit(stdout);
        // else: 300s timeout with no mention — just re-arm immediately.
        continue;
      }

      // Non-zero: likely "no active session" during the startup race (before the
      // SessionStart hook's session-start has completed). Back off, then re-arm.
      log("wait-for-mention exited", code, stderr ? `— ${stderr}` : "");
      await Bun.sleep(ERROR_BACKOFF_MS);
    } catch (e) {
      // A throw must NEVER kill the loop.
      log("re-arm loop error", e);
      await Bun.sleep(ERROR_BACKOFF_MS);
    }
  }
}

// ── boot ──────────────────────────────────────────────────────────────────────

// Connect FIRST (sets up the stdio wire), then start consuming mentions — mirrors
// the docs starting Bun.serve after mcp.connect().
await mcp.connect(new StdioServerTransport());
log("connected over stdio");
rearmLoop(); // fire-and-forget; runs for the life of the process
