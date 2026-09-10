import assert from "node:assert/strict"
import test from "node:test"

import { caller, piProfileAgent } from "../protocol.ts"

function fakeCtx() {
  return {
    sessionManager: { getSessionId: () => "sess-1" },
    cwd: "/tmp/work",
  } as never
}

function withEnv(vars: Record<string, string | undefined>, run: () => void) {
  const saved = Object.fromEntries(
    Object.keys(vars).map((key) => [key, process.env[key]]),
  )
  for (const [key, value] of Object.entries(vars)) {
    if (value === undefined) delete process.env[key]
    else process.env[key] = value
  }
  try {
    run()
  } finally {
    for (const [key, value] of Object.entries(saved)) {
      if (value === undefined) delete process.env[key]
      else process.env[key] = value
    }
  }
}

test("profile agent slug is always <profile>-pi", () => {
  assert.equal(piProfileAgent("/Users/x/.pi/architect"), "architect-pi")
  assert.equal(piProfileAgent("/Users/x/.pi/agent"), "agent-pi")
  assert.equal(piProfileAgent("/Users/x/custom"), "custom-pi")
})

test("unhosted caller claims the profile-derived agent slug", () => {
  withEnv(
    {
      MOSAICO_OBSERVED_HARNESS: undefined,
      MOSAICO_AGENT: undefined,
      PI_CODING_AGENT_DIR: "/Users/x/.pi/architect",
    },
    () => {
      assert.equal(caller(fakeCtx()).agent, "architect-pi")
    },
  )
})

test("unhosted default profile claims agent-pi", () => {
  withEnv(
    {
      MOSAICO_OBSERVED_HARNESS: undefined,
      MOSAICO_AGENT: undefined,
      PI_CODING_AGENT_DIR: undefined,
      HOME: "/Users/x",
    },
    () => {
      assert.equal(caller(fakeCtx()).agent, "agent-pi")
    },
  )
})

test("hosted caller keeps the launch-owned agent slug", () => {
  withEnv(
    {
      MOSAICO_OBSERVED_HARNESS: "pi",
      MOSAICO_AGENT: "writer",
      PI_CODING_AGENT_DIR: "/Users/x/.pi/architect",
    },
    () => {
      assert.equal(caller(fakeCtx()).agent, "writer")
    },
  )
})

test("hosted caller without a launch agent falls back to the harness slug", () => {
  withEnv(
    {
      MOSAICO_OBSERVED_HARNESS: "pi",
      MOSAICO_AGENT: undefined,
      PI_CODING_AGENT_DIR: "/Users/x/.pi/architect",
    },
    () => {
      assert.equal(caller(fakeCtx()).agent, "pi")
    },
  )
})