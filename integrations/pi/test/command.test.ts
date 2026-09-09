import assert from "node:assert/strict"
import test from "node:test"
import { registerMosaicoCommand, renderOverview } from "../command.ts"

const snapshot = {
  running: true,
  name: "drift-pi",
  state: "working",
  title: "Complete Pi integration",
  channels: [{ path: "#mosaico", about: "Main coordination" }],
  error: "",
}

test("renders the current session name, status, and active channels", () => {
  assert.equal(renderOverview(snapshot), [
    "Mosaico running",
    "@drift-pi · working",
    "Working on: Complete Pi integration",
    "Channels: #mosaico",
  ].join("\n"))
})

test("registers only the join completion and renders the default overview", async () => {
  const { command, notifications } = harness()
  assert.deepEqual(command.getArgumentCompletions("jo"), [{ value: "join", label: "join" }])

  await command.handler("", context(notifications))

  assert.equal(notifications.length, 1)
  assert.match(notifications[0][0], /@drift-pi · working/)
})

test("joins an explicitly named channel", async () => {
  const joined: string[] = []
  const { command, notifications } = harness({ joined })

  await command.handler("join #pi/tools", context(notifications))

  assert.deepEqual(joined, ["#pi/tools"])
  assert.deepEqual(notifications, [["Joined #pi/tools.", "info"]])
})

test("selects any known channel that is not already active", async () => {
  const joined: string[] = []
  const selections: Array<[string, string[]]> = []
  const { command, notifications } = harness({ joined })
  const ctx = context(notifications, async (title, options) => {
    selections.push([title, options])
    return "#pi/tools — Pi integration"
  })

  await command.handler("join", ctx)

  assert.deepEqual(selections, [["Join a Mosaico channel", [
    "#pi — Pi workspace",
    "#pi/tools — Pi integration",
  ]]])
  assert.deepEqual(joined, ["#pi/tools"])
  assert.deepEqual(notifications, [["Joined #pi/tools.", "info"]])
})

test("cancelling the channel selector does not join", async () => {
  const joined: string[] = []
  const { command, notifications } = harness({ joined })

  await command.handler("join", context(notifications, async () => undefined))

  assert.deepEqual(joined, [])
  assert.deepEqual(notifications, [])
})

function harness(overrides: { joined?: string[] } = {}) {
  let command: any
  const notifications: Array<[string, string]> = []
  const joined = overrides.joined || []
  registerMosaicoCommand({
    registerCommand(name: string, definition: unknown) {
      assert.equal(name, "mosaico")
      command = definition
    },
  } as never, {
    readSnapshot: async () => snapshot,
    listChannels: async () => [
      { path: "#mosaico", about: "Main coordination" },
      { path: "#pi", about: "Pi workspace" },
      { path: "#pi/tools", about: "Pi integration" },
    ],
    joinChannel: async channel => {
      joined.push(channel)
      return `Joined ${channel}.`
    },
  })
  return { command, notifications }
}

function context(
  notifications: Array<[string, string]>,
  select: (title: string, options: string[]) => Promise<string | undefined> = async () => undefined,
) {
  return {
    ui: {
      notify: (message: string, level: string) => notifications.push([message, level]),
      select,
    },
  }
}
