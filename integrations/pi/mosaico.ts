import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent"
import { DeliveryPump } from "./delivery.ts"
import { caller, daemonCall, isHostedPi } from "./protocol.ts"
import { clearSessionStatus, paintSessionStatus } from "./status.ts"
import { registerMosaicoTools } from "./tools.ts"

const HOOK_TIMEOUT_MS = 5_000
const REGISTER_TIMEOUT_MS = 15_000
const boundaryWarnings = new Map<string, string>()

export default function mosaico(pi: ExtensionAPI) {
  let pump: DeliveryPump | undefined
  registerMosaicoTools(pi)

  pi.on("session_start", async (_event, ctx) => {
    if (await register(ctx)) {
      pump = new DeliveryPump(pi, ctx)
      pump.start()
    }
    await paintSessionStatus(ctx)
  })

  pi.on("before_agent_start", async (event, ctx) => {
    const registered = await register(ctx)
    if (registered) pump?.start()
    const context = registered
      ? await turnContext("turn_start", ctx, { prompt: event.prompt })
      : fabricUnavailable()
    await paintSessionStatus(ctx)
    return context ? customContext(context) : undefined
  })

  pi.on("tool_call", async (event, ctx) => {
    const request = directPath(event.toolName, event.input)
    if (!request) return
    try {
      const result = await daemonCall("cross_project_path_classify", {
        ...caller(ctx),
        ...request,
      }, AbortSignal.timeout(HOOK_TIMEOUT_MS)) as { decision?: string; message?: string }
      if (result.decision === "deny" && result.message) return { block: true, reason: result.message }
      if (result.decision === "warn" && result.message) boundaryWarnings.set(event.toolCallId, result.message)
    } catch {
      // Guardrails are cooperative and fail open when the daemon is unavailable.
    }
  })

  pi.on("tool_result", async (event, ctx) => {
    const warning = boundaryWarnings.get(event.toolCallId)
    boundaryWarnings.delete(event.toolCallId)
    const context = await turnContext("turn_check", ctx)
    const addition = [warning, context].filter(Boolean).join("\n\n")
    return addition ? { content: [{ type: "text", text: addition }, ...event.content] } : undefined
  })

  pi.on("message_start", event => pump?.onMessageStart(event))

  pi.on("agent_settled", async (_event, ctx) => {
    if (!(isHostedPi() && process.env.MOSAICO_TRANSPORT === "pi-rpc")) {
      await daemonCall("turn_end", caller(ctx), AbortSignal.timeout(HOOK_TIMEOUT_MS)).catch(() => undefined)
    }
    await paintSessionStatus(ctx)
  })

  pi.on("session_shutdown", async (_event, ctx) => {
    pump?.stop()
    pump = undefined
    await daemonCall("session_end", {
      ...caller(ctx),
      cause: "harness_hook",
    }, AbortSignal.timeout(HOOK_TIMEOUT_MS)).catch(() => undefined)
    clearSessionStatus(ctx)
  })
}

async function register(ctx: ExtensionContext): Promise<boolean> {
  const hosted = isHostedPi()
  const transport = hosted ? process.env.MOSAICO_TRANSPORT || "" : ""
  try {
    await daemonCall("session_start", {
      ...caller(ctx),
      claimed_harness: "pi",
      observed_harness: "pi",
      admitted_transport: transport,
      endpoint_provenance: "hook",
      pubkey: hosted ? process.env.MOSAICO_PUBKEY || undefined : undefined,
    }, AbortSignal.timeout(REGISTER_TIMEOUT_MS))
    return true
  } catch {
    return false
  }
}

async function turnContext(
  method: "turn_start" | "turn_check",
  ctx: ExtensionContext,
  extra: Record<string, unknown> = {},
): Promise<string> {
  try {
    const result = await daemonCall(method, { ...caller(ctx), ...extra }, AbortSignal.timeout(HOOK_TIMEOUT_MS))
    return contextFrom(result)
  } catch {
    return ""
  }
}

function contextFrom(result: unknown): string {
  const context = (result as { context?: unknown }).context
  return typeof context === "string" ? context : ""
}

function customContext(content: string) {
  return { message: { customType: "mosaico.context", content, display: false } }
}

function fabricUnavailable(): string {
  return "<mosaico>\n⚠ Fabric temporarily unavailable — this session could not be registered with the daemon, so its inbox and channel awareness may be incomplete. Do not assume the channel is quiet.\n</mosaico>"
}

function directPath(toolName: string, input: unknown): { access: "read" | "write"; path: string } | undefined {
  const paths = input as Record<string, unknown>
  const access = ["read", "glob", "grep", "view_image", "read_file", "readmediafile"].includes(toolName.toLowerCase())
    ? "read"
    : ["write", "edit", "multiedit", "notebookedit", "write_file"].includes(toolName.toLowerCase())
      ? "write"
      : undefined
  if (!access || !paths || typeof paths !== "object") return undefined
  for (const key of ["file_path", "filePath", "path", "notebook_path", "notebookPath"]) {
    if (typeof paths[key] === "string" && paths[key]) return { access, path: paths[key] }
  }
  return undefined
}
