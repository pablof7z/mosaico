import type { ExtensionContext } from "@earendil-works/pi-coding-agent"
import { execFile } from "node:child_process"

export type Params = Record<string, unknown>
export type Details = Record<string, unknown> | null

export interface ProtocolResult {
  content: Array<{ type: "text"; text: string }>
  details: Details
  isError: boolean
}

export function protocolError(message: string): ProtocolResult {
  return {
    content: [{ type: "text", text: message }],
    details: { error: message },
    isError: true,
  }
}

export function execute(
  bin: string,
  tool: string,
  params: Params,
  signal: AbortSignal | undefined,
  ctx: ExtensionContext,
): Promise<ProtocolResult> {
  return new Promise(resolve => {
    let settled = false
    const finish = (result: ProtocolResult) => {
      if (settled) return
      settled = true
      signal?.removeEventListener("abort", onAbort)
      resolve(result)
    }
    const child = execFile(bin, ["harness", "pi"], {
      cwd: ctx.cwd,
      maxBuffer: 8 * 1024 * 1024,
    }, (error, stdout, stderr) => {
      if (error) {
        finish(protocolError(
          stderr.trim() || `Mosaico ${tool} transport failed: ${error.message}`,
        ))
        return
      }
      try {
        const result = JSON.parse(stdout) as ProtocolResult
        if (!Array.isArray(result.content) || typeof result.isError !== "boolean") {
          throw new Error("response is not a Pi tool result")
        }
        finish(result)
      } catch (parseError) {
        const reason = parseError instanceof Error ? parseError.message : String(parseError)
        finish(protocolError(`Mosaico ${tool} returned invalid JSON: ${reason}`))
      }
    })
    const onAbort = () => {
      child.kill()
      finish(protocolError(`Mosaico ${tool} was aborted.`))
    }
    signal?.addEventListener("abort", onAbort, { once: true })
    if (signal?.aborted) onAbort()
    child.stdin?.end(JSON.stringify({
      version: 1,
      tool,
      arguments: params,
      session: {
        native_id: ctx.sessionManager.getSessionId(),
        cwd: ctx.cwd,
        pty_session: process.env.MOSAICO_PTY_SESSION || undefined,
      },
    }))
  })
}
