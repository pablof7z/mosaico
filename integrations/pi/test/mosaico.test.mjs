import assert from "node:assert/strict"
import test from "node:test"

import mosaico from "../mosaico.ts"
import { daemonFixture, eventually, holdDeliveryWait } from "./daemon-fixture.mjs"

const context = {
  cwd: "/workspace",
  sessionManager: { getSessionId: () => "pi-native-session" },
}

function install({ transport = "pty", observedHarness = "pi", agent = "pi", pubkey } = {}) {
  const handlers = new Map()
  const tools = new Map()
  const sent = []
  const pi = {
    on: (name, handler) => handlers.set(name, handler),
    registerTool: tool => tools.set(tool.name, tool),
    sendMessage: (message, options) => {
      sent.push({ message, options })
      queueMicrotask(() => handlers.get("message_start")?.({
        message: { role: "custom", customType: message.customType, details: message.details },
      }))
    },
  }
  const inherited = Object.fromEntries([
    "MOSAICO_AGENT", "MOSAICO_OBSERVED_HARNESS", "MOSAICO_PTY_SESSION", "MOSAICO_TRANSPORT", "MOSAICO_PUBKEY",
  ].map(key => [key, process.env[key]]))
  process.env.MOSAICO_AGENT = agent
  process.env.MOSAICO_OBSERVED_HARNESS = observedHarness
  delete process.env.MOSAICO_PTY_SESSION
  process.env.MOSAICO_TRANSPORT = transport
  if (pubkey === undefined) delete process.env.MOSAICO_PUBKEY
  else process.env.MOSAICO_PUBKEY = pubkey
  mosaico(pi)
  return { handlers, inherited, sent, tools }
}

async function shutdown(instance) {
  await instance.handlers.get("session_shutdown")({}, context)
  for (const [key, value] of Object.entries(instance.inherited)) {
    if (value === undefined) delete process.env[key]
    else process.env[key] = value
  }
}

test("registers Pi's native identity and injects context through the daemon UDS", async () => {
  const daemon = await daemonFixture(frame => {
    if (frame.method === "turn_start") return { ok: { context: "<mosaico>aware</mosaico>" } }
    return holdDeliveryWait(frame) || { ok: { pubkey: "agent-pubkey" } }
  })
  const pi = install()
  try {
    await pi.handlers.get("session_start")({}, context)
    const injected = await pi.handlers.get("before_agent_start")({ prompt: "hello" }, context)
    await pi.handlers.get("agent_settled")({}, context)

    assert.equal(injected.message.customType, "mosaico.context")
    assert.equal(injected.message.content, "<mosaico>aware</mosaico>")
    const start = daemon.requests.find(request => request.method === "session_start")
    assert.deepEqual(start.params.harness_session, "pi-native-session")
    assert.equal(start.params.harness, "pi")
    assert.equal(start.params.cwd, "/workspace")
    assert.equal(start.params.claimed_harness, "pi")
    assert.equal(start.params.observed_harness, "pi")
    assert.deepEqual(
      daemon.requests.map(request => request.method).filter(method => method !== "session_delivery_wait"),
      ["session_start", "session_start", "turn_start", "turn_end"],
    )
  } finally {
    await shutdown(pi)
    await daemon.close()
  }
})

test("managed Pi RPC retains its transport-owned completion and delivery", async () => {
  const daemon = await daemonFixture(frame => {
    if (frame.method === "turn_start") return { ok: { context: "context" } }
    return { ok: { pubkey: "agent-pubkey" } }
  })
  const pi = install({ transport: "pi-rpc" })
  try {
    await pi.handlers.get("session_start")({}, context)
    await pi.handlers.get("before_agent_start")({ prompt: "hello" }, context)
    await pi.handlers.get("agent_settled")({}, context)
    assert.equal(daemon.requests.some(request => request.method === "session_delivery_wait"), false)
    assert.equal(daemon.requests.some(request => request.method === "turn_end"), false)
  } finally {
    await shutdown(pi)
    await daemon.close()
  }
})

test("a manually launched Pi ignores inherited parent-host identity but remains directly reachable", async () => {
  const daemon = await daemonFixture(frame => holdDeliveryWait(frame) || { ok: { pubkey: "manual-pubkey" } })
  const pi = install({ transport: "pi-rpc", observedHarness: "codex", agent: "codex", pubkey: "parent-pubkey" })
  try {
    await pi.handlers.get("session_start")({}, context)
    await eventually(() => assert.ok(daemon.requests.some(request => request.method === "session_delivery_wait")))
    const start = daemon.requests.find(request => request.method === "session_start")
    assert.equal(start.params.agent, "pi")
    assert.equal(start.params.admitted_transport, "")
    assert.equal("pubkey" in start.params, false)
    await pi.handlers.get("agent_settled")({}, context)
    assert.ok(daemon.requests.some(request => request.method === "turn_end"))
  } finally {
    await shutdown(pi)
    await daemon.close()
  }
})

test("exposes only agent-facing Mosaico tools and keeps calls structured", async () => {
  const daemon = await daemonFixture(frame => {
    if (frame.method === "pi_tool_call") return { ok: {
      content: [{ type: "text", text: "ok" }], details: { accepted: true }, is_error: false,
    } }
    return holdDeliveryWait(frame) || { ok: { pubkey: "agent-pubkey" } }
  })
  const pi = install()
  try {
    assert.deepEqual([...pi.tools.keys()], [
      "mosaico_session", "mosaico_wait", "mosaico_channel_list", "mosaico_channel_read",
      "mosaico_channel_search", "mosaico_send", "mosaico_reply", "mosaico_react",
      "mosaico_channel_create", "mosaico_channel_join", "mosaico_channel_leave", "mosaico_dispatch",
    ])
    const result = await pi.tools.get("mosaico_reply").execute(
      "id", { message_id: "event", message: "done" }, undefined, undefined, context,
    )
    assert.equal(result.isError, false)
    const call = daemon.requests.find(request => request.method === "pi_tool_call")
    assert.deepEqual(call.params, {
      harness: "pi", harness_session: "pi-native-session", cwd: "/workspace",
      watch_pid: process.pid, agent: "pi",
      tool: "mosaico_reply", arguments: { message_id: "event", message: "done" },
    })
  } finally {
    await shutdown(pi)
    await daemon.close()
  }
})

test("channel reads use a daemon stream, including duration timestamps", async () => {
  const daemon = await daemonFixture(frame => {
    if (frame.method === "channel_read") return [
      { item: { event_id: "one", body: "hello" } }, { end: true },
    ]
    return holdDeliveryWait(frame) || { ok: { pubkey: "agent-pubkey" } }
  })
  const pi = install()
  try {
    const result = await pi.tools.get("mosaico_channel_read").execute(
      "id", { channel: "#work", since: "2h", limit: 3 }, undefined, undefined, context,
    )
    assert.equal(result.isError, false)
    assert.deepEqual(result.details, { messages: [{ event_id: "one", body: "hello" }] })
    const call = daemon.requests.find(request => request.method === "channel_read")
    assert.equal(call.params.since, "2h")
    assert.equal(call.params.tail, true)
    assert.equal(call.params.live, false)
  } finally {
    await shutdown(pi)
    await daemon.close()
  }
})

test("a direct delivery becomes one custom Pi message and is acknowledged after acceptance", async () => {
  let waits = 0
  const daemon = await daemonFixture(frame => {
    if (frame.method !== "session_delivery_wait") return { ok: { pubkey: "agent-pubkey" } }
    waits += 1
    if (waits === 1) return { ok: {
      kind: "delivery", lease_id: "lease-1",
      message: { custom_type: "mosaico.delivery", content: "direct message", display: false, details: { event_ids: ["event-1"] } },
    } }
    return new Promise(() => {})
  })
  const pi = install()
  try {
    await pi.handlers.get("session_start")({}, context)
    await eventually(() => assert.equal(pi.sent.length, 1))
    await eventually(() => assert.ok(daemon.requests.some(request => request.method === "session_delivery_ack")))
    assert.deepEqual(pi.sent[0], {
      message: {
        customType: "mosaico.delivery", content: "direct message", display: false,
        details: { event_ids: ["event-1"], lease_id: "lease-1" },
      },
      options: { triggerTurn: true, deliverAs: "steer" },
    })
    const ack = daemon.requests.find(request => request.method === "session_delivery_ack")
    assert.equal(ack.params.lease_id, "lease-1")
    assert.equal(ack.params.accepted, true)
  } finally {
    await shutdown(pi)
    await daemon.close()
  }
})

test("daemon failures and aborted requests become explicit Pi tool errors", async () => {
  let release
  const blocked = new Promise(resolve => { release = resolve })
  const daemon = await daemonFixture(async frame => {
    if (frame.method === "pi_tool_call") return blocked
    return holdDeliveryWait(frame) || { ok: { pubkey: "agent-pubkey" } }
  })
  const pi = install()
  try {
    const controller = new AbortController()
    const result = pi.tools.get("mosaico_wait").execute(
      "id", { timeout_seconds: 1 }, controller.signal, undefined, context,
    )
    await eventually(() => assert.ok(daemon.requests.some(request => request.method === "pi_tool_call")))
    controller.abort()
    const output = await result
    assert.equal(output.isError, true)
    assert.match(output.content[0].text, /aborted/i)
  } finally {
    release()
    await shutdown(pi)
    await daemon.close()
  }
})
