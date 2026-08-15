import type {
  ExtensionAPI,
  ExtensionContext,
} from "@earendil-works/pi-coding-agent"
import { execFile } from "node:child_process"
import { existsSync } from "node:fs"
import { homedir } from "node:os"
import { join } from "node:path"
import { registerMosaicoTools } from "./tools.ts"
import { clearSessionStatus, paintSessionStatus } from "./status.ts"

function resolveBin(): string {
  if (process.env.MOSAICO_BIN) return process.env.MOSAICO_BIN
  for (const candidate of [
    join(homedir(), ".local", "bin", "mosaico"),
    "/usr/local/bin/mosaico",
  ]) {
    if (existsSync(candidate)) return candidate
  }
  return "mosaico"
}

export default function mosaico(pi: ExtensionAPI) {
  const bin = resolveBin()
  const boundaryWarnings = new Map<string, string>()

  registerMosaicoTools(pi, bin)

  function runHook(
    type: string,
    payload: Record<string, unknown>,
    timeout = 60_000,
  ): Promise<string> {
    return new Promise((resolve) => {
      const child = execFile(
        bin,
        ["harness", "hook", "pi", "--type", type],
        { timeout, maxBuffer: 8 * 1024 * 1024 },
        (_error, stdout) => resolve(stdout ?? ""),
      )
      child.stdin?.end(JSON.stringify(payload))
    })
  }

  function session(ctx: ExtensionContext) {
    return {
      session_id: ctx.sessionManager.getSessionId(),
      resume_id: ctx.sessionManager.getSessionId(),
      cwd: ctx.cwd,
      pid: process.pid,
      transport: process.env.MOSAICO_TRANSPORT ?? "",
      endpoint_id: process.env.MOSAICO_ENDPOINT_ID ?? "",
    }
  }

  pi.on("session_start", async (_event, ctx) => {
    await runHook("session-start", session(ctx), 5_000)
    await paintSessionStatus(ctx, bin)
  })

  pi.on("before_agent_start", async (event, ctx) => {
    const context = (
      await runHook("user-prompt-submit", {
        ...session(ctx),
        prompt: event.prompt,
      })
    ).trim()
    await paintSessionStatus(ctx, bin)
    if (!context) return
    return {
      message: {
        customType: "mosaico",
        content: context,
        display: false,
      },
    }
  })

  pi.on("tool_call", async (event, ctx) => {
    const raw = await runHook(
      "pre-tool-use",
      {
        ...session(ctx),
        tool_name: event.toolName,
        tool_input: event.input,
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
      return { block: true, reason: result.message }
    }
    if (result.decision === "warn" && result.message) {
      boundaryWarnings.set(event.toolCallId, result.message)
    }
  })

  pi.on("tool_result", async (event, ctx) => {
    const warning = boundaryWarnings.get(event.toolCallId) ?? ""
    boundaryWarnings.delete(event.toolCallId)
    const context = (await runHook("post-tool-use", session(ctx))).trim()
    const addition = [warning, context].filter(Boolean).join("\n\n")
    if (!addition) return
    return {
      content: [{ type: "text", text: addition }, ...event.content],
    }
  })

  pi.on("agent_settled", async (_event, ctx) => {
    if ((process.env.MOSAICO_TRANSPORT ?? "") !== "pi-rpc") {
      await runHook("stop", session(ctx), 5_000)
    }
    await paintSessionStatus(ctx, bin)
  })

  pi.on("session_shutdown", async (_event, ctx) => {
    await runHook("session-end", session(ctx), 5_000)
    clearSessionStatus(ctx)
  })
}
