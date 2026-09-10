import assert from "node:assert/strict"
import test from "node:test"
import { panelSnapshot } from "../panel-model.ts"

test("the sidebar snapshot contains only the current session's scoped context", () => {
  const snapshot = panelSnapshot({
    running: true,
    overview: {
      name: "drift-pi\u001b[31m",
      state: "working",
      title: "Build the focused Pi panel\u0007",
      status_age: "4 min ago",
      channels: [
        { path: "#mosaico", about: "Main coordination" },
        { path: "#mosaico/panel", about: "Panel work" },
      ],
      interactions: [{
        name: "haven-codex",
        channel: "#mosaico",
        age: "2 min ago",
        message: "The latest exchange\u0007",
      }],
    },
  })

  assert.deepEqual(snapshot.session, {
    name: "drift-pi",
    state: "working",
    title: "Build the focused Pi panel",
    statusAge: "4 min ago",
  })
  assert.deepEqual(snapshot.channels, [
    { path: "#mosaico", about: "Main coordination" },
    { path: "#mosaico/panel", about: "Panel work" },
  ])
  assert.deepEqual(snapshot.interactions, [{
    name: "haven-codex",
    channel: "#mosaico",
    age: "2 min ago",
    message: "The latest exchange",
  }])
  assert.equal(Object.hasOwn(snapshot, "relays"), false)
  assert.equal(Object.hasOwn(snapshot, "sessions"), false)
  assert.equal(Object.hasOwn(snapshot, "activity"), false)
  assert.doesNotThrow(() => JSON.stringify(snapshot))
  assert.equal(hasUndefined(snapshot), false)
})

test("malformed and empty daemon values stay empty without undefined", () => {
  const snapshot = panelSnapshot({ running: true, overview: { channels: [undefined], interactions: [{}] } })
  assert.deepEqual(snapshot.channels, [])
  assert.deepEqual(snapshot.interactions, [])
  assert.equal(hasUndefined(snapshot), false)
})

function hasUndefined(value: unknown): boolean {
  if (value === undefined) return true
  if (!value || typeof value !== "object") return false
  return Object.values(value).some(hasUndefined)
}
