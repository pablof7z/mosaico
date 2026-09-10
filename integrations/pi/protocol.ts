import type { ExtensionContext } from "@earendil-works/pi-coding-agent"
import { daemonCall, mosaicoHome, type Params } from "./daemon.ts"
import { basename, join } from "node:path"
import { homedir } from "node:os"

export { DaemonConnection, daemonCall, mosaicoHome, type Params } from "./daemon.ts"

export type Details = Record<string, unknown> | null

export interface ProtocolResult {
  content: Array<{ type: "text"; text: string }>
  details: Details
  isError: boolean
}

/**
 * The fabric agent slug for the pi profile this process runs under: always
 * `<profile>-pi` so a session handle reads `<codename>-<profile>-pi` (e.g.
 * `architect-pi` for `~/.pi/architect`, `agent-pi` for the default profile).
 * Pi selects the profile directory via `PI_CODING_AGENT_DIR`.
 */
export function piProfileAgent(
  agentDir: string = process.env.PI_CODING_AGENT_DIR || join(homedir(), ".pi", "agent"),
): string {
  const profile = basename(agentDir) || "agent"
  return `${profile}-pi`
}

export function caller(ctx: ExtensionContext): Params {
  const hosted = process.env.MOSAICO_OBSERVED_HARNESS === "pi"
  return {
    harness: "pi",
    harness_session: ctx.sessionManager.getSessionId(),
    cwd: ctx.cwd,
    watch_pid: process.pid,
    agent: hosted ? process.env.MOSAICO_AGENT || "pi" : piProfileAgent(),
  }
}

/** True only for a process Mosaico itself launched as Pi. */
export function isHostedPi(): boolean {
  return process.env.MOSAICO_OBSERVED_HARNESS === "pi"
}

export async function execute(
  tool: string,
  params: Params,
  signal: AbortSignal | undefined,
  ctx: ExtensionContext,
): Promise<ProtocolResult> {
  try {
    const details = await daemonCall("pi_tool_call", { ...caller(ctx), tool, arguments: params }, signal)
    return parseResult(tool, details)
  } catch (error) {
    return protocolError(`Mosaico ${tool} failed: ${message(error)}`)
  }
}

export function protocolError(message: string): ProtocolResult {
  return { content: [{ type: "text", text: message }], details: { error: message }, isError: true }
}

function parseResult(tool: string, value: unknown): ProtocolResult {
  const result = value as Partial<{ content: ProtocolResult["content"]; details: Details; is_error: boolean }>
  if (!Array.isArray(result.content) || typeof result.is_error !== "boolean") {
    return protocolError(`Mosaico ${tool} returned an invalid daemon result.`)
  }
  return { content: result.content, details: result.details ?? null, isError: result.is_error }
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
