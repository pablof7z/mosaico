import assert from "node:assert/strict"
import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import mosaico from "../mosaico.ts"

function fakeMosaico() {
  const root = mkdtempSync(join(tmpdir(), "pi-mosaico-test-"))
  const log = join(root, "hooks.jsonl")
  const bin = join(root, "mosaico")
  writeFileSync(
    bin,
    `#!/usr/bin/env node
const { appendFileSync } = require("node:fs")
let input = ""
process.stdin.setEncoding("utf8")
process.stdin.on("data", chunk => { input += chunk })
process.stdin.on("end", () => {
  const index = process.argv.indexOf("--type")
  const type = index >= 0 ? process.argv[index + 1] : ""
  appendFileSync(process.env.PI_MOSAICO_TEST_LOG, JSON.stringify({
    type,
    payload: input ? JSON.parse(input) : null,
  }) + "\\n")
  if (type === "user-prompt-submit") process.stdout.write("fabric context")
})
`,
  )
  chmodSync(bin, 0o755)
  return {
    bin,
    log,
    cleanup: () => rmSync(root, { recursive: true, force: true }),
    calls: () => {
      try {
        return readFileSync(log, "utf8")
          .trim()
          .split("\n")
          .filter(Boolean)
          .map(line => JSON.parse(line))
      } catch (error) {
        if (error.code === "ENOENT") return []
        throw error
      }
    },
  }
}

function register(transport, endpointId) {
  const handlers = new Map()
  const pi = { on: (name, handler) => handlers.set(name, handler) }
  process.env.MOSAICO_TRANSPORT = transport
  process.env.MOSAICO_ENDPOINT_ID = endpointId
  mosaico(pi)
  return handlers
}

const context = {
  cwd: "/workspace",
  sessionManager: { getSessionId: () => "pi-session" },
}

test("PTY owns lifecycle and publishes endpoint correlation", async () => {
  const fake = fakeMosaico()
  process.env.MOSAICO_BIN = fake.bin
  process.env.PI_MOSAICO_TEST_LOG = fake.log
  try {
    const handlers = register("pty", "pty-endpoint")
    assert.equal(handlers.has("agent_end"), false)
    assert.equal(handlers.has("agent_settled"), true)

    await handlers.get("session_start")({}, context)
    const injected = await handlers.get("before_agent_start")(
      { prompt: "hello" },
      context,
    )
    await handlers.get("agent_settled")({}, context)

    assert.equal(injected.message.content, "fabric context")
    const calls = fake.calls()
    assert.deepEqual(calls.map(call => call.type), [
      "session-start",
      "user-prompt-submit",
      "stop",
    ])
    assert.equal(calls[0].payload.transport, "pty")
    assert.equal(calls[0].payload.endpoint_id, "pty-endpoint")
  } finally {
    fake.cleanup()
    delete process.env.MOSAICO_BIN
    delete process.env.PI_MOSAICO_TEST_LOG
    delete process.env.MOSAICO_TRANSPORT
    delete process.env.MOSAICO_ENDPOINT_ID
  }
})

test("managed Pi RPC owns lifecycle while extension injects context", async () => {
  const fake = fakeMosaico()
  process.env.MOSAICO_BIN = fake.bin
  process.env.PI_MOSAICO_TEST_LOG = fake.log
  try {
    const handlers = register("pi-rpc", "rpc-endpoint")
    assert.equal(handlers.has("agent_settled"), true)

    await handlers.get("session_start")({}, context)
    const injected = await handlers.get("before_agent_start")(
      { prompt: "hello" },
      context,
    )
    await handlers.get("agent_settled")({}, context)

    assert.equal(injected.message.content, "fabric context")
    const calls = fake.calls()
    assert.deepEqual(calls.map(call => call.type), [
      "session-start",
      "user-prompt-submit",
    ])
    assert.equal(calls[0].payload.transport, "pi-rpc")
    assert.equal(calls[0].payload.endpoint_id, "rpc-endpoint")
  } finally {
    fake.cleanup()
    delete process.env.MOSAICO_BIN
    delete process.env.PI_MOSAICO_TEST_LOG
    delete process.env.MOSAICO_TRANSPORT
    delete process.env.MOSAICO_ENDPOINT_ID
  }
})
