import { mkdtempSync, rmSync } from "node:fs"
import { createServer } from "node:net"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { once } from "node:events"

export async function daemonFixture(handle) {
  const home = mkdtempSync(join(tmpdir(), "pi-mosaico-daemon-"))
  const socketPath = join(home, "daemon.sock")
  const sockets = new Set()
  const requests = []
  const server = createServer(socket => {
    sockets.add(socket)
    socket.setEncoding("utf8")
    let buffer = ""
    socket.on("close", () => sockets.delete(socket))
    socket.on("data", chunk => {
      buffer += chunk
      for (;;) {
        const newline = buffer.indexOf("\n")
        if (newline < 0) return
        const line = buffer.slice(0, newline)
        buffer = buffer.slice(newline + 1)
        if (!line) continue
        const frame = JSON.parse(line)
        if (typeof frame.protocol === "number") {
          socket.write(`${JSON.stringify({ protocol: 72, daemon_version: "test" })}\n`)
          continue
        }
        requests.push(frame)
        void Promise.resolve(handle(frame, socket, requests)).then(reply => {
          if (!reply || socket.destroyed) return
          for (const value of Array.isArray(reply) ? reply : [reply]) {
            socket.write(`${JSON.stringify({ id: frame.id, ...value })}\n`)
          }
        })
      }
    })
  })
  server.listen(socketPath)
  await once(server, "listening")
  const oldHome = process.env.MOSAICO_HOME
  process.env.MOSAICO_HOME = home
  return {
    requests,
    async close() {
      for (const socket of sockets) socket.destroy()
      await new Promise(resolve => server.close(resolve))
      if (oldHome === undefined) delete process.env.MOSAICO_HOME
      else process.env.MOSAICO_HOME = oldHome
      rmSync(home, { recursive: true, force: true })
    },
  }
}

export async function eventually(assertion, timeoutMs = 1_000) {
  const until = Date.now() + timeoutMs
  let last
  while (Date.now() < until) {
    try {
      return assertion()
    } catch (error) {
      last = error
      await new Promise(resolve => setTimeout(resolve, 10))
    }
  }
  throw last
}

export function holdDeliveryWait(frame) {
  if (frame.method === "session_delivery_wait") return new Promise(() => {})
  return undefined
}
