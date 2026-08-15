import type { ExtensionContext } from "@earendil-works/pi-coding-agent"
import { createConnection, type Socket } from "node:net"
import { homedir } from "node:os"
import { join } from "node:path"

const PROTOCOL = 72
const CLIENT_VERSION = "pi-mosaico"

export type Params = Record<string, unknown>
export type Details = Record<string, unknown> | null

export interface ProtocolResult {
  content: Array<{ type: "text"; text: string }>
  details: Details
  isError: boolean
}

interface Response {
  id: number
  ok?: unknown
  error?: { message?: string }
  item?: unknown
  end?: boolean
}

interface Pending {
  id: number
  items: unknown[]
  resolve: (value: unknown) => void
  reject: (error: Error) => void
}

export class DaemonConnection {
  private buffer = ""
  private handshake?: { resolve: () => void; reject: (error: Error) => void }
  private pending?: Pending
  private nextId = 1
  private readonly socket: Socket

  private constructor(socket: Socket) {
    this.socket = socket
    socket.setEncoding("utf8")
    socket.on("data", (chunk: string) => this.read(chunk))
    socket.on("error", error => this.fail(error))
    socket.on("close", () => this.fail(new Error("Mosaico daemon connection closed")))
  }

  static async open(signal?: AbortSignal): Promise<DaemonConnection> {
    const socket = createConnection(daemonSocketPath())
    const connection = new DaemonConnection(socket)
    await connection.open(signal)
    return connection
  }

  async call(method: string, params: Params, signal?: AbortSignal): Promise<unknown> {
    if (this.pending) throw new Error("Mosaico daemon connection already has a request")
    const id = this.nextId++
    return new Promise((resolve, reject) => {
      const onAbort = () => {
        this.close()
        reject(new Error(`Mosaico ${method} was aborted.`))
      }
      signal?.addEventListener("abort", onAbort, { once: true })
      this.pending = {
        id,
        items: [],
        resolve: value => {
          signal?.removeEventListener("abort", onAbort)
          resolve(value)
        },
        reject: error => {
          signal?.removeEventListener("abort", onAbort)
          reject(error)
        },
      }
      this.write({ id, method, params })
    })
  }

  close(): void {
    this.socket.destroy()
  }

  private open(signal?: AbortSignal): Promise<void> {
    return new Promise((resolve, reject) => {
      const onAbort = () => {
        this.close()
        reject(new Error("Mosaico daemon connection was aborted."))
      }
      signal?.addEventListener("abort", onAbort, { once: true })
      this.handshake = {
        resolve: () => {
          signal?.removeEventListener("abort", onAbort)
          resolve()
        },
        reject: error => {
          signal?.removeEventListener("abort", onAbort)
          reject(error)
        },
      }
      this.socket.once("connect", () => this.write({ protocol: PROTOCOL, client_version: CLIENT_VERSION }))
    })
  }

  private read(chunk: string): void {
    this.buffer += chunk
    for (;;) {
      const newline = this.buffer.indexOf("\n")
      if (newline < 0) return
      const line = this.buffer.slice(0, newline)
      this.buffer = this.buffer.slice(newline + 1)
      if (!line) continue
      let frame: Response & { protocol?: number }
      try {
        frame = JSON.parse(line) as Response & { protocol?: number }
      } catch {
        this.fail(new Error("Mosaico daemon sent invalid JSON."))
        return
      }
      if (this.handshake) {
        if (frame.protocol !== PROTOCOL) {
          this.handshake.reject(new Error("Mosaico daemon protocol is incompatible; restart Pi after updating Mosaico."))
        } else {
          this.handshake.resolve()
        }
        this.handshake = undefined
        continue
      }
      this.handle(frame)
    }
  }

  private handle(frame: Response): void {
    const pending = this.pending
    if (!pending || frame.id !== pending.id) return
    if (frame.error) {
      this.pending = undefined
      pending.reject(new Error(frame.error.message || "Mosaico daemon request failed."))
      return
    }
    if (frame.item !== undefined) {
      pending.items.push(frame.item)
      return
    }
    if (frame.end) {
      this.pending = undefined
      pending.resolve(pending.items)
      return
    }
    if (frame.ok !== undefined) {
      this.pending = undefined
      pending.resolve(frame.ok)
    }
  }

  private fail(reason: Error): void {
    const handshake = this.handshake
    this.handshake = undefined
    handshake?.reject(reason)
    const pending = this.pending
    this.pending = undefined
    pending?.reject(reason)
  }

  private write(frame: object): void {
    this.socket.write(`${JSON.stringify(frame)}\n`)
  }
}

export async function daemonCall(
  method: string,
  params: Params,
  signal?: AbortSignal,
): Promise<unknown> {
  const connection = await DaemonConnection.open(signal)
  try {
    return await connection.call(method, params, signal)
  } finally {
    connection.close()
  }
}

export async function daemonStream(
  method: string,
  params: Params,
  signal?: AbortSignal,
): Promise<unknown[]> {
  const result = await daemonCall(method, params, signal)
  return Array.isArray(result) ? result : []
}

export function caller(ctx: ExtensionContext): Params {
  const hosted = process.env.MOSAICO_OBSERVED_HARNESS === "pi"
  return {
    harness: "pi",
    harness_session: ctx.sessionManager.getSessionId(),
    cwd: ctx.cwd,
    watch_pid: process.pid,
    agent: hosted ? process.env.MOSAICO_AGENT || "pi" : "pi",
  }
}

/** True only for a process Mosaico itself launched as Pi, never an inherited
 * parent-shell environment from another harness. */
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
    const details = await daemonCall("pi_tool_call", {
      ...caller(ctx),
      tool,
      arguments: params,
    }, signal)
    return parseResult(tool, details)
  } catch (error) {
    return protocolError(`Mosaico ${tool} failed: ${message(error)}`)
  }
}

export async function readChannel(
  params: Params,
  signal: AbortSignal | undefined,
  ctx: ExtensionContext,
): Promise<ProtocolResult> {
  try {
    const items = await daemonStream("channel_read", {
      ...caller(ctx),
      id: stringOrUndefined(params.id),
      channel: stringOrUndefined(params.channel),
      since: params.since,
      limit: numberOrUndefined(params.limit),
      offset: 0,
      tail: true,
      live: false,
    }, signal)
    const details = { messages: items }
    return {
      content: [{ type: "text", text: JSON.stringify(details, null, 2) }],
      details,
      isError: false,
    }
  } catch (error) {
    return protocolError(`Mosaico mosaico_channel_read failed: ${message(error)}`)
  }
}

export function protocolError(message: string): ProtocolResult {
  return {
    content: [{ type: "text", text: message }],
    details: { error: message },
    isError: true,
  }
}

function parseResult(tool: string, value: unknown): ProtocolResult {
  const result = value as Partial<{ content: ProtocolResult["content"]; details: Details; is_error: boolean }>
  if (!Array.isArray(result.content) || typeof result.is_error !== "boolean") {
    return protocolError(`Mosaico ${tool} returned an invalid daemon result.`)
  }
  return { content: result.content, details: result.details ?? null, isError: result.is_error }
}

function daemonSocketPath(): string {
  return join(process.env.MOSAICO_HOME || join(homedir(), ".mosaico"), "daemon.sock")
}

function stringOrUndefined(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value : undefined
}

function numberOrUndefined(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
