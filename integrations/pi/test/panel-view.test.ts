import assert from "node:assert/strict"
import test from "node:test"
import createSidebarView from "../panel-view.ts"
import type { PanelSnapshot } from "../panel-model.ts"

test("the TUI renders only the current session card", () => {
  const state: PanelSnapshot = {
    running: true,
    session: {
      name: "falcon-pi",
      state: "working",
      title: "E2E smoke test of fabric basics",
      statusAge: "6 min ago",
    },
    channels: [
      { path: "#mosaico", about: "Main coordination" },
      { path: "#mosaico/testing", about: "Fabric smoke tests" },
    ],
    interactions: [{
      name: "haven-codex",
      channel: "#mosaico/testing",
      age: "2 min ago",
      message: "Please verify the latest smoke result",
    }],
    error: "",
    updatedAt: 0,
  }
  const theme = { fg: (_name: string, value: string) => value, bold: (value: string) => value }
  const output = createSidebarView({ getState: () => state, theme }).render(80).join("\n")

  assert.match(output, /^@falcon-pi/m)
  assert.match(output, /E2E smoke test of fabric basics/)
  assert.match(output, /6 min ago/)
  assert.match(output, /#mosaico.*Main coordination/)
  assert.match(output, /\|- #testing.*Fabric smoke tests/)
  assert.match(output, /@haven-codex on #mosaico\/testing.*2 min ago/)
  assert.match(output, /Please verify the latest smoke result/)
  assert.doesNotMatch(output, /Mosaico|running|Working on|Interacted with|Relay|Sessions/)
})

test("empty healthy sections and ordinary lifecycle state render nothing", () => {
  const state: PanelSnapshot = {
    running: true,
    session: { name: "falcon-pi", state: "idle", title: "", statusAge: "" },
    channels: [],
    interactions: [],
    error: "",
    updatedAt: 0,
  }
  const theme = { fg: (_name: string, value: string) => value, bold: (value: string) => value }
  const output = createSidebarView({ getState: () => state, theme }).render(80).join("\n").trim()
  assert.equal(output, "@falcon-pi")
})

test("failure state is visible without restoring a healthy Mosaico banner", () => {
  const state: PanelSnapshot = {
    running: true,
    session: { name: "falcon-pi", state: "offline", title: "", statusAge: "" },
    channels: [],
    interactions: [],
    error: "",
    updatedAt: 0,
  }
  const theme = { fg: (_name: string, value: string) => value, bold: (value: string) => value }
  const output = createSidebarView({ getState: () => state, theme }).render(80).join("\n").trim()
  assert.equal(output, "@falcon-pi · offline")
  assert.doesNotMatch(output, /Mosaico|running/)
})
