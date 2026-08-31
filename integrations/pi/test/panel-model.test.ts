import assert from "node:assert/strict"
import test from "node:test"
import { activityRow, panelSnapshot } from "../panel-model.ts"

test("the sidebar snapshot excludes inactive sessions and serializes cleanly", () => {
  const snapshot = panelSnapshot({
    home: "/tmp/mosaico",
    running: true,
    relays: ["wss://relay.example/"],
    sessions: { sessions: [
      { agent: "codex", harness: "pi", running: true, state: "working", workspaces: [{ channels: [{ name: "#mosaico/panel" }] }] },
      { agent: "old", running: false },
    ] },
    channels: { sections: [{ channels: [{ path: "#mosaico", children: [{ path: "#mosaico/panel", agents: 2 }] }] }] },
  })
  assert.deepEqual(snapshot.sessions, [{ agent: "codex", harness: "pi", channel: "#mosaico/panel", state: "working" }])
  assert.deepEqual(snapshot.channels.map(channel => channel.path), ["#mosaico", "#mosaico/panel"])
  assert.equal(snapshot.relays[0]?.label, "relay.example")
  assert.doesNotThrow(() => JSON.stringify(snapshot))
})

test("the sidebar strips terminal control sequences from daemon activity", () => {
  assert.deepEqual(activityRow({ category: "msg", channel: "#root", from: "a\u001b[31m", body: "hi\u0007" }), {
    category: "msg", channel: "#root", text: "a  hi",
  })
})
