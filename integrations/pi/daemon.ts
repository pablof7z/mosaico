import { createConnection, type Socket } from "node:net"
import { homedir } from "node:os"
import { join } from "node:path"

const PROTOCOL = 72
const CLIENT_VERSION = "pi-mosaico"

export type Params = Record<string, unknown>

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

interface Subscription {
  id: number
  onItem: (item: unknown) => void
  onEnd: (error?: Error) => void
}

export class DaemonConnection {
  private buffer = ""
  private handshake?: { resolve: () => void; reject: (error: Error) => void }
  private pending?: Pending
  private subscription?: Subscription
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
    const connection = new DaemonConnection(createConnection(daemonSocketPath()))
    await connection.open(signal)
    return connection
  }

  async call(method: string, params: Params, signal?: AbortSignal): Promise<unknown> {
    if (this.pending || this.subscription) throw new Error("Mosaico daemon connection already has a request")
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

  subscribe(method: string, params: Params, onItem: (item: unknown) => void, onEnd: (error?: Error) => void): () => void {
    if (this.pending || this.subscription) throw new Error("Mosaico daemon connection already has a request")
    const id = this.nextId++
    this.subscription = { id, onItem, onEnd }
    this.write({ id, method, params })
    return () => this.close()
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
    const subscription = this.subscription
    if (subscription && frame.id === subscription.id) {
      if (frame.item !== undefined) return subscription.onItem(frame.item)
      if (frame.error || frame.end) {
        this.subscription = undefined
        subscription.onEnd(frame.error ? new Error(frame.error.message || "Mosaico daemon request failed.") : undefined)
      }
      return
    }
    const pending = this.pending
    if (!pending || frame.id !== pending.id) return
    if (frame.error) {
      this.pending = undefined
      pending.reject(new Error(frame.error.message || "Mosaico daemon request failed."))
    } else if (frame.item !== undefined) {
      pending.items.push(frame.item)
    } else if (frame.end) {
      this.pending = undefined
      pending.resolve(pending.items)
    } else if (frame.ok !== undefined) {
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
    const subscription = this.subscription
    this.subscription = undefined
    subscription?.onEnd(reason)
  }

  private write(frame: object): void {
    this.socket.write(`${JSON.stringify(frame)}\n`)
  }
}

export async function daemonCall(method: string, params: Params, signal?: AbortSignal): Promise<unknown> {
  const connection = await DaemonConnection.open(signal)
  try {
    return await connection.call(method, params, signal)
  } finally {
    connection.close()
  }
}

export async function daemonStream(method: string, params: Params, signal?: AbortSignal): Promise<unknown[]> {
  const result = await daemonCall(method, params, signal)
  return Array.isArray(result) ? result : []
}

export function mosaicoHome(): string {
  return process.env.MOSAICO_HOME || join(homedir(), ".mosaico")
}

function daemonSocketPath(): string {
  return join(mosaicoHome(), "daemon.sock")
}
