import assert from "node:assert/strict"
import { chmodSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import mosaico from "../mosaico.ts"

function fakeMosaico() {
  const root = mkdtempSync(join(tmpdir(), "pi-mosaico-test-"))
  const log = join(root, "hooks.jsonl")
  const bin = join(root, "mosaico")
  const program = join(root, "mosaico.cjs")
  writeFileSync(
    program,
    `
const { appendFileSync } = require("node:fs")
let input = ""
process.stdin.setEncoding("utf8")
process.stdin.on("data", chunk => { input += chunk })
process.stdin.on("end", () => {
  if (process.argv.slice(2).join(" ") === "harness pi") {
    const request = JSON.parse(input)
    appendFileSync(process.env.PI_MOSAICO_TEST_LOG, JSON.stringify({
      type: "pi-tool",
      payload: request,
    }) + "\\n")
    if (process.env.PI_MOSAICO_TEST_INVALID === "1") {
      process.stdout.write("not-json")
      return
    }
    const reply = () => process.stdout.write(JSON.stringify({
      content: [{ type: "text", text: "structured tool result" }],
      details: { tool: request.tool, arguments: request.arguments },
      isError: false,
    }))
    if (process.env.PI_MOSAICO_TEST_DELAY === "1") setTimeout(reply, 10000)
    else reply()
    return
  }
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
  writeFileSync(
    bin,
    `#!/bin/sh
exec ${JSON.stringify(process.execPath)} ${JSON.stringify(program)} "$@"
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
  const tools = new Map()
  const pi = {
    on: (name, handler) => handlers.set(name, handler),
    registerTool: tool => tools.set(tool.name, tool),
  }
  process.env.MOSAICO_TRANSPORT = transport
  process.env.MOSAICO_ENDPOINT_ID = endpointId
  mosaico(pi)
  return { handlers, tools }
}

const context = {
  cwd: "/workspace",
  sessionManager: { getSessionId: () => "pi-session" },
}
const toolContext = {
  ...context,
  cwd: tmpdir(),
}

test("PTY owns lifecycle and publishes endpoint correlation", async () => {
  const fake = fakeMosaico()
  process.env.MOSAICO_BIN = fake.bin
  process.env.PI_MOSAICO_TEST_LOG = fake.log
  try {
    const { handlers } = register("pty", "pty-endpoint")
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
    const { handlers } = register("pi-rpc", "rpc-endpoint")
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

test("registers only native agent-facing Mosaico tools", () => {
  const { tools } = register("pty", "pty-endpoint")
  assert.deepEqual([...tools.keys()], [
    "mosaico_session",
    "mosaico_wait",
    "mosaico_channel_list",
    "mosaico_channel_read",
    "mosaico_channel_search",
    "mosaico_send",
    "mosaico_reply",
    "mosaico_react",
    "mosaico_channel_create",
    "mosaico_channel_join",
    "mosaico_channel_leave",
    "mosaico_dispatch",
  ])
  for (const forbidden of ["daemon", "setup", "agents", "kill", "resume"]) {
    assert.equal([...tools.keys()].some(name => name.includes(forbidden)), false)
  }
})

test("tool execution uses the structured harness pi protocol", async () => {
  const fake = fakeMosaico()
  process.env.MOSAICO_BIN = fake.bin
  process.env.PI_MOSAICO_TEST_LOG = fake.log
  process.env.MOSAICO_PTY_SESSION = "pty-session"
  try {
    assert.equal(existsSync(fake.bin), true)
    const { tools } = register("pty", "pty-endpoint")
    const result = await tools.get("mosaico_reply").execute(
      "tool-call",
      { message_id: "abcd", message: "done" },
      undefined,
      undefined,
      toolContext,
    )

    assert.equal(result.isError, false, JSON.stringify(result))
    assert.equal(result.details.tool, "mosaico_reply")
    assert.deepEqual(result.details.arguments, { message_id: "abcd", message: "done" })
    const request = fake.calls().find(call => call.type === "pi-tool").payload
    assert.equal(request.version, 1)
    assert.equal(request.session.native_id, "pi-session")
    assert.equal(request.session.cwd, tmpdir())
    assert.equal(request.session.public_session, undefined)
    assert.equal(request.session.pty_session, "pty-session")
  } finally {
    fake.cleanup()
    delete process.env.MOSAICO_BIN
    delete process.env.PI_MOSAICO_TEST_LOG
    delete process.env.MOSAICO_PTY_SESSION
  }
})

test("invalid protocol output is an explicit Pi tool error", async () => {
  const fake = fakeMosaico()
  process.env.MOSAICO_BIN = fake.bin
  process.env.PI_MOSAICO_TEST_LOG = fake.log
  process.env.PI_MOSAICO_TEST_INVALID = "1"
  try {
    const { tools } = register("pty", "pty-endpoint")
    const result = await tools.get("mosaico_session").execute(
      "tool-call", {}, undefined, undefined, toolContext,
    )
    assert.equal(result.isError, true)
    assert.match(result.content[0].text, /returned invalid JSON/)
  } finally {
    fake.cleanup()
    delete process.env.MOSAICO_BIN
    delete process.env.PI_MOSAICO_TEST_LOG
    delete process.env.PI_MOSAICO_TEST_INVALID
  }
})

test("tool cancellation aborts the harness process", async () => {
  const fake = fakeMosaico()
  process.env.MOSAICO_BIN = fake.bin
  process.env.PI_MOSAICO_TEST_LOG = fake.log
  process.env.PI_MOSAICO_TEST_DELAY = "1"
  try {
    const { tools } = register("pty", "pty-endpoint")
    const controller = new AbortController()
    const pending = tools.get("mosaico_wait").execute(
      "tool-call", { timeout_seconds: 30 }, controller.signal, undefined, toolContext,
    )
    controller.abort()
    const result = await pending
    assert.equal(result.isError, true)
    assert.match(result.content[0].text, /aborted|transport failed/i)
  } finally {
    fake.cleanup()
    delete process.env.MOSAICO_BIN
    delete process.env.PI_MOSAICO_TEST_LOG
    delete process.env.PI_MOSAICO_TEST_DELAY
  }
})
