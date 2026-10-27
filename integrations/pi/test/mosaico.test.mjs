import assert from "node:assert/strict"
import test from "node:test"

import mosaico from "../mosaico.ts"
import { daemonFixture, eventually, holdDeliveryWait } from "./daemon-fixture.mjs"

const context = {
  cwd: "/workspace",
  sessionManager: { getSessionId: () => "pi-native-session" },
}

async function install({ transport = "pty", observedHarness = "pi", agent = "pi", pubkey, agentSessionName } = {}) {
  const handlers = new Map()
  const commands = new Map()
  const tools = new Map()
  const sent = []
  const pi = {
    on: (name, handler) => handlers.set(name, handler),
    registerCommand: (name, command) => commands.set(name, command),
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
    "PI_CODING_AGENT_DIR", "AGENT_SESSION_NAME",
  ].map(key => [key, process.env[key]]))
  process.env.MOSAICO_AGENT = agent
  process.env.MOSAICO_OBSERVED_HARNESS = observedHarness
  delete process.env.MOSAICO_PTY_SESSION
  process.env.MOSAICO_TRANSPORT = transport
  process.env.PI_CODING_AGENT_DIR = "/tmp/pi-mosaico-test/.pi/agent"
  if (pubkey === undefined) delete process.env.MOSAICO_PUBKEY
  else process.env.MOSAICO_PUBKEY = pubkey
  if (agentSessionName === undefined) delete process.env.AGENT_SESSION_NAME
  else process.env.AGENT_SESSION_NAME = agentSessionName
  await mosaico(pi)
  return { commands, handlers, inherited, sent, tools }
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
  const pi = await install()
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
      ["session_start", "session_delivery_open", "session_start", "turn_start", "turn_end"],
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
  const pi = await install({ transport: "pi-rpc" })
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
  const pi = await install({ transport: "pi-rpc", observedHarness: "codex", agent: "codex", pubkey: "parent-pubkey" })
  try {
    await pi.handlers.get("session_start")({}, context)
    await eventually(() => assert.ok(daemon.requests.some(request => request.method === "session_delivery_wait")))
    const start = daemon.requests.find(request => request.method === "session_start")
    assert.equal(start.params.agent, "agent-pi")
    assert.equal(start.params.admitted_transport, "")
    assert.equal("pubkey" in start.params, false)
    assert.ok(daemon.requests.some(request => request.method === "session_delivery_open"))
    await pi.handlers.get("agent_settled")({}, context)
    assert.ok(daemon.requests.some(request => request.method === "turn_end"))
  } finally {
    await shutdown(pi)
    await daemon.close()
  }
})

test("an unhosted Pi forwards AGENT_SESSION_NAME as a soft session_name_hint", async () => {
  const daemon = await daemonFixture(frame => holdDeliveryWait(frame) || { ok: { pubkey: "manual-pubkey" } })
  const pi = await install({
    transport: "", observedHarness: "codex", agent: "codex", agentSessionName: "orchestrator-child-7",
  })
  try {
    await pi.handlers.get("session_start")({}, context)
    const start = daemon.requests.find(request => request.method === "session_start")
    assert.equal(start.params.session_name_hint, "orchestrator-child-7")
  } finally {
    await shutdown(pi)
    await daemon.close()
  }
})

test("a Mosaico-hosted Pi never forwards AGENT_SESSION_NAME as a hint", async () => {
  const daemon = await daemonFixture(frame => holdDeliveryWait(frame) || { ok: { pubkey: "agent-pubkey" } })
  const pi = await install({ agentSessionName: "should-be-ignored" })
  try {
    await pi.handlers.get("session_start")({}, context)
    const start = daemon.requests.find(request => request.method === "session_start")
    assert.equal("session_name_hint" in start.params, false)
  } finally {
    await shutdown(pi)
    await daemon.close()
  }
})

test("a Pi extension establishes its wake path after daemon registration recovers", async () => {
  let starts = 0
  const daemon = await daemonFixture(frame => {
    if (frame.method === "session_start" && starts++ === 0) {
      return { error: { code: "rpc_error", message: "temporarily unavailable" } }
    }
    if (frame.method === "turn_start") return { ok: { context: "" } }
    return holdDeliveryWait(frame) || { ok: { pubkey: "manual-pubkey" } }
  })
  const pi = await install()
  try {
    await pi.handlers.get("session_start")({}, context)
    assert.equal(daemon.requests.some(request => request.method === "session_delivery_open"), false)

    await pi.handlers.get("before_agent_start")({ prompt: "hello" }, context)

    assert.ok(daemon.requests.some(request => request.method === "session_delivery_open"))
    await eventually(() => assert.ok(daemon.requests.some(request => request.method === "session_delivery_wait")))
  } finally {
    await shutdown(pi)
    await daemon.close()
  }
})

test("exposes agent-facing Mosaico tools and keeps calls structured", async () => {
  const daemon = await daemonFixture(frame => {
    if (frame.method === "pi_tool_call") return { ok: {
      content: [{ type: "text", text: "ok" }], details: { accepted: true }, is_error: false,
    } }
    return holdDeliveryWait(frame) || { ok: { pubkey: "agent-pubkey" } }
  })
  const pi = await install()
  try {
    assert.deepEqual([...pi.commands.keys()], ["mosaico"])
    assert.deepEqual([...pi.tools.keys()], [
      "mosaico_session", "mosaico_wait", "mosaico_channel_list", "mosaico_channel_read",
      "mosaico_channel_search", "mosaico_send", "mosaico_reply", "mosaico_react",
      "mosaico_channel_create", "mosaico_channel_join", "mosaico_channel_leave", "mosaico_dispatch",
    ])
    const result = await pi.tools.get("mosaico_reply").execute(
      "id", { id: "abc123", message: "done" }, undefined, undefined, context,
    )
    assert.equal(result.isError, false)
    const call = daemon.requests.find(request => request.method === "pi_tool_call")
    assert.deepEqual(call.params, {
      harness: "pi", harness_session: "pi-native-session", cwd: "/workspace",
      watch_pid: process.pid, agent: "pi",
      tool: "mosaico_reply", arguments: { id: "abc123", message: "done" },
    })
  } finally {
    await shutdown(pi)
    await daemon.close()
  }
})

test("/mosaico reads the daemon through the native protocol", async () => {
  const daemon = await daemonFixture(frame => {
    if (frame.method === "my_session_overview") return { ok: {
      name: "drift-pi",
      state: "working",
      title: "Complete Pi integration",
      channels: [{ path: "#mosaico", about: "Main coordination" }],
      interactions: [{
        name: "haven-codex",
        channel: "#mosaico",
        age: "2 min ago",
        message: "Latest exchange",
      }],
    } }
    return holdDeliveryWait(frame) || { ok: { pubkey: "agent-pubkey" } }
  })
  const pi = await install()
  const notifications = []
  try {
    await pi.commands.get("mosaico").handler("", {
      ...context,
      ui: { notify: (message, level) => notifications.push({ message, level }) },
    })
    assert.match(notifications[0].message, /Mosaico running/)
    assert.match(notifications[0].message, /@drift-pi · working/)
    assert.match(notifications[0].message, /Working on: Complete Pi integration/)
    assert.equal(notifications[0].level, "info")
    assert.deepEqual(
      daemon.requests.slice(-1).map(request => request.method),
      ["my_session_overview"],
    )
  } finally {
    await shutdown(pi)
    await daemon.close()
  }
})

test("/mosaico join selects and joins a known channel through the native protocol", async () => {
  const daemon = await daemonFixture(frame => {
    if (frame.method === "my_session_overview") return { ok: {
      name: "drift-pi", state: "working", title: "Complete Pi integration",
      channels: [{ path: "#mosaico", about: "Main coordination" }], interactions: [],
    } }
    if (frame.method === "channel_list") {
      return { ok: { sections: [{ channels: [
          { path: "#mosaico", about: "Main coordination", children: [] },
          { path: "#pi", about: "Pi workspace", children: [
            { path: "#pi/tools", about: "Pi integration", children: [] },
          ] },
        ] }] } }
    }
    if (frame.method === "channel_join") {
      return { ok: { channel: frame.params.channel } }
    }
    return holdDeliveryWait(frame) || { ok: { pubkey: "agent-pubkey" } }
  })
  const pi = await install()
  const notifications = []
  try {
    await pi.commands.get("mosaico").handler("join", {
      ...context,
      ui: {
        notify: (message, level) => notifications.push({ message, level }),
        select: async (_title, options) => {
          assert.deepEqual(options, ["#pi — Pi workspace", "#pi/tools — Pi integration"])
          return "#pi/tools — Pi integration"
        },
      },
    })
    assert.deepEqual(notifications, [{ message: "Joined #pi/tools.", level: "info" }])
    const calls = daemon.requests.filter(request => ["channel_list", "channel_join"].includes(request.method))
    assert.deepEqual(calls.map(call => [call.method, call.params.channel, call.params.recursive]), [
      ["channel_list", undefined, true],
      ["channel_join", "#pi/tools", undefined],
    ])
  } finally {
    await shutdown(pi)
    await daemon.close()
  }
})

test("channel reads route through pi_tool_call and return plain text rows", async () => {
  const daemon = await daemonFixture(frame => {
    if (frame.method === "pi_tool_call" && frame.params.tool === "mosaico_channel_read") return { ok: {
      content: [{ type: "text", text: "<@sender> hello [1m ago]" }], is_error: false,
    } }
    return holdDeliveryWait(frame) || { ok: { pubkey: "agent-pubkey" } }
  })
  const pi = await install()
  try {
    const result = await pi.tools.get("mosaico_channel_read").execute(
      "id", { channel: "#work", since: "2h", limit: 3 }, undefined, undefined, context,
    )
    assert.equal(result.isError, false)
    assert.equal(result.details, null)
    const call = daemon.requests.find(
      request => request.method === "pi_tool_call" && request.params.tool === "mosaico_channel_read"
    )
    assert.ok(call)
    assert.equal(call.params.arguments.channel, "#work")
    assert.equal(call.params.arguments.since, "2h")
    assert.equal(call.params.arguments.limit, 3)
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
      message: { custom_type: "mosaico.delivery", content: "direct message", details: { event_ids: ["event-1"] } },
    } }
    return new Promise(() => {})
  })
  const pi = await install()
  try {
    await pi.handlers.get("session_start")({}, context)
    await eventually(() => assert.equal(pi.sent.length, 1))
    await eventually(() => assert.ok(daemon.requests.some(request => request.method === "session_delivery_ack")))
    assert.deepEqual(pi.sent[0], {
      message: {
        customType: "mosaico.delivery", content: "direct message", display: true,
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

test("tool_result turn_check context is sent via sendMessage, not prepended to tool result", async () => {
  const daemon = await daemonFixture(frame => {
    if (frame.method === "turn_check") return { ok: { context: "<mosaico>mid-turn</mosaico>" } }
    return holdDeliveryWait(frame) || { ok: { pubkey: "agent-pubkey" } }
  })
  const pi = await install()
  try {
    await pi.handlers.get("session_start")({}, context)
    const result = await pi.handlers.get("tool_result")({
      toolCallId: "call-1",
      content: [{ type: "text", text: "tool output" }],
    }, context)
    // No boundary warning → tool result unmodified
    assert.equal(result, undefined)
    // Context delivered via sendMessage with display: false (debug off by default)
    const contextMsg = pi.sent.find(s => s.message.customType === "mosaico.context")
    assert.ok(contextMsg, "sendMessage must have been called with mosaico.context")
    assert.equal(contextMsg.message.content, "<mosaico>mid-turn</mosaico>")
    assert.equal(contextMsg.message.display, false)
    assert.equal(contextMsg.options, undefined)
  } finally {
    await shutdown(pi)
    await daemon.close()
  }
})

test("tool_result boundary warning stays in tool result even when context is sent separately", async () => {
  const daemon = await daemonFixture(frame => {
    if (frame.method === "cross_project_path_classify") return { ok: { decision: "warn", message: "cross-project boundary" } }
    if (frame.method === "turn_check") return { ok: { context: "<mosaico>check</mosaico>" } }
    return holdDeliveryWait(frame) || { ok: { pubkey: "agent-pubkey" } }
  })
  const pi = await install()
  try {
    await pi.handlers.get("session_start")({}, context)
    await pi.handlers.get("tool_call")({
      toolCallId: "call-2", toolName: "read", input: { file_path: "/other/project/file.ts" },
    }, context)
    const result = await pi.handlers.get("tool_result")({
      toolCallId: "call-2",
      content: [{ type: "text", text: "file content" }],
    }, context)
    // Warning prepended to tool result; original content follows
    assert.ok(result, "tool result must be modified when a warning exists")
    assert.equal(result.content[0].text, "cross-project boundary")
    assert.equal(result.content[1].text, "file content")
    // Context still sent via sendMessage
    const contextMsg = pi.sent.find(s => s.message.customType === "mosaico.context")
    assert.ok(contextMsg)
  } finally {
    await shutdown(pi)
    await daemon.close()
  }
})

test("/mosaico debug toggles context display on then off", async () => {
  const daemon = await daemonFixture(frame => {
    if (frame.method === "turn_check") return { ok: { context: "<mosaico>check</mosaico>" } }
    return holdDeliveryWait(frame) || { ok: { pubkey: "agent-pubkey" } }
  })
  const pi = await install()
  const notifications = []
  const ui = { notify: (message, level) => notifications.push({ message, level }) }
  try {
    await pi.handlers.get("session_start")({}, context)

    // Toggle ON
    await pi.commands.get("mosaico").handler("debug", { ...context, ui })
    assert.match(notifications[0].message, /debug on/i)
    assert.equal(notifications[0].level, "info")

    // Context message now has display: true
    await pi.handlers.get("tool_result")({
      toolCallId: "call-1", content: [{ type: "text", text: "out" }],
    }, context)
    const onMsg = pi.sent.find(s => s.message.customType === "mosaico.context")
    assert.ok(onMsg)
    assert.equal(onMsg.message.display, true)

    // Toggle OFF — leaves state clean for other tests
    await pi.commands.get("mosaico").handler("debug", { ...context, ui })
    assert.match(notifications[1].message, /debug off/i)
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
  const pi = await install()
  try {
    const controller = new AbortController()
    const result = pi.tools.get("mosaico_wait").execute(
      "id", { seconds: 1 }, controller.signal, undefined, context,
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
