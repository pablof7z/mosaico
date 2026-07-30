import type { Plugin } from "@opencode-ai/plugin"
import { execFile } from "node:child_process"
import { existsSync } from "node:fs"
import { homedir } from "node:os"
import { join } from "node:path"

// ── mosaico ⇄ opencode bridge ─────────────────────────────────────────────
//
// Makes an opencode session a citizen on the mosaico fabric. Lifecycle steps
// all go through the single `hook` entry point (same as Claude Code / Codex) —
// this plugin pipes a JSON payload on stdin and reads stdout back:
//   session-start (hook)        → on first message of a session (spawns the
//                   presence engine, watching opencode's PID so it reaps on exit)
//   user-prompt-submit (hook)   → experimental.chat.messages.transform, on the
//                   first model invocation of a user message (marks "working",
//                   then injects its stdout — self-identity, workspace chat,
//                   peer roster, all assembled by
//                   the shared Rust hook — is injected verbatim into the turn.
//   post-tool-use (hook)        → experimental.chat.messages.transform, on later
//                   model invocations of the same message (mid-turn checkpoint:
//                   peeks new messages + sibling deltas; same stdout-as-context).
//   stop (hook)                 → event → session.idle (marks the session idle)
//
// The plugin never builds context strings itself: the hook is the single source
// of truth, identical to Claude Code / Codex. We pipe its stdout into the turn.
//
// mosaico knows nothing about opencode; this plugin is the straw.
// Env: MOSAICO_BIN (path), MOSAICO_AGENT (slug, default "opencode").

function resolveBin(): string {
  if (process.env.MOSAICO_BIN) return process.env.MOSAICO_BIN
  for (const c of [join(homedir(), ".local", "bin", "mosaico"), "/usr/local/bin/mosaico"]) {
    if (existsSync(c)) return c
  }
  return "mosaico"
}

// MOSAICO_CONTEXT_HELPERS_START
/**
 * Replace Mosaico-owned parts on one retained message with at most one current part.
 * @param {{parts: Array<any>}} message
 * @param {any} [current]
 */
export function replaceMosaicoPart(message, current) {
  message.parts = message.parts.filter((part) => !part?._mosaicoInjected)
  if (current) message.parts.unshift(current)
}

/**
 * Drain queued tool-boundary warnings exactly once.
 * @param {Array<string>} queue
 */
export function drainBoundaryWarnings(queue) {
  return queue.splice(0).join("\n\n")
}
// MOSAICO_CONTEXT_HELPERS_END

export const Mosaico: Plugin = async ({ directory }) => {
  const BIN = resolveBin()

  // Session/turn lifecycle goes through the single `hook` entry point — the same
  // door Claude Code and Codex use — by piping a small JSON payload on stdin and
  // reading stdout back. mosaico has no per-step subcommands anymore; `harness
  // hook opencode --type <t>` parses this payload and drives the lifecycle.
  function runHook(
    type: string,
    payload: Record<string, unknown>,
    timeout = 60_000,
  ): Promise<string> {
    return new Promise((resolve) => {
      const child = execFile(
        BIN,
        ["harness", "hook", "opencode", "--type", type],
        { timeout, maxBuffer: 8 * 1024 * 1024 },
        (_e, out) => resolve(out ?? ""),
      )
      child.stdin?.end(JSON.stringify(payload))
    })
  }

  // Start the session in the background (fire-and-forget) so plugin load NEVER
  // blocks opencode startup.
  // Watch opencode's PID so the engine reaps + goes idle when opencode exits.
  // No native session locator exists at plugin startup. We pass our PID so the
  // daemon has a typed lifecycle locator until the first turn. The agent
  // slug comes from the inherited MOSAICO_AGENT env (default "opencode").
  runHook("session-start", { cwd: directory, pid: process.pid }).catch(() => {})

  // Turn bracketing. The transform handler fires once per *model invocation*
  // (i.e. many times per user turn in an agentic loop), so turn-start is gated
  // to fire only when the latest user message id changes. We also remember the
  // opencode session id so session.idle can close the current turn.
  let lastTurnMsgID = ""
  let ocSessionForTurn = ""
  const pendingBoundaryWarnings: string[] = []

  return {
    // Cooperative cross-project guard: only Rust recognizes known direct-path
    // tools. Empty, malformed, unavailable, and unsupported results allow.
    "tool.execute.before": async (input, output) => {
      const raw = await runHook(
        "pre-tool-use",
        {
          session_id: input.sessionID,
          cwd: directory,
          tool_name: input.tool,
          tool_input: output.args,
        },
        5_000,
      )
      let result: { decision?: string; message?: string } = {}
      try {
        result = JSON.parse(raw)
      } catch {
        return
      }
      if (result.decision === "deny" && result.message) {
        throw new Error(result.message)
      }
      if (result.decision === "warn" && result.message) {
        pendingBoundaryWarnings.push(result.message)
      }
    },

    // Inject peer mentions before the model sees the turn, and (once per user
    // message) mark the turn "working".
    "experimental.chat.messages.transform": async (_input, output) => {
      const msgs = output.messages as Array<{ info: any; parts: any[] }>
      // A transform receives retained message history. Remove every prior
      // Mosaico part first, even when this invocation has no current context.
      for (const message of msgs) {
        replaceMosaicoPart(message)
      }
      let lastUser: { info: any; parts: any[] } | undefined
      for (let i = msgs.length - 1; i >= 0; i--) {
        if (msgs[i].info?.role === "user") { lastUser = msgs[i]; break }
      }
      if (!lastUser) return
      const msgID: string = lastUser.info.id
      const ocSessionID: string = lastUser.info.sessionID
      ocSessionForTurn = ocSessionID

      // The hook is the single source of truth for injected context: the same
      // Rust path that serves Claude Code and Codex assembles the self-identity
      // line, project chat, and peer roster, and prints it on stdout. We don't
      // rebuild any of that here — we just pipe stdout into the turn. Two hook
      // types map to opencode's two moments:
      //   • new user message → user-prompt-submit (turn start: drains chat,
      //     full roster on the first turn, deltas after).
      //   • same user message, later model invocation → post-tool-use (mid-turn
      //     checkpoint: non-destructive peek of new chat + sibling deltas,
      //     rate-limited by the daemon — exactly Claude Code's PostToolUse).
      let context = ""
      if (ocSessionID && msgID && msgID !== lastTurnMsgID) {
        lastTurnMsgID = msgID
        // We deliberately omit `prompt`, so (unlike Claude Code / Codex) the
        // prompt is NOT published as a kind:1 OP — preserving prior behavior.
        // The opencode id is a harness locator and resume token, never identity.
        context = (
          await runHook("user-prompt-submit", {
            session_id: ocSessionID,
            resume_id: ocSessionID,
          })
        ).trim()
      } else if (ocSessionID) {
        context = (await runHook("post-tool-use", { session_id: ocSessionID })).trim()
      }
      if (pendingBoundaryWarnings.length) {
        const warnings = drainBoundaryWarnings(pendingBoundaryWarnings)
        context = context ? `${warnings}\n\n${context}` : warnings
      }

      if (context) {
        replaceMosaicoPart(lastUser, {
          id: `mosaico-${msgID}`,
          sessionID: ocSessionID,
          messageID: msgID,
          type: "text",
          text: context,
          _mosaicoInjected: true,
        } as any)
      }
    },

    // Turn finished: opencode emits session.idle when the assistant is done
    // responding. Mark the session idle using opencode's native locator.
    event: async ({ event }: { event: any }) => {
      if (event?.type === "session.idle") {
        const ocSessionID = String(event?.properties?.sessionID ?? ocSessionForTurn ?? "")
        if (ocSessionID) await runHook("stop", { session_id: ocSessionID })
      }
    },
  }
}
