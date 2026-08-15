import assert from "node:assert/strict"
import test from "node:test"

import { parseSessionStatus, renderSessionStatus } from "../status.ts"

const fabric = `
<mosaico>
  <self name="@cliff-pi" host="Pablos-MacBook-Pro-3" headless="off" unhosted="true" workspace="mosaico" branch="master" title="Add session status chrome to pi-mosaico" />
</mosaico>
`

test("parses the current session from the mosaico_session snapshot", () => {
  assert.deepEqual(parseSessionStatus(fabric), {
    name: "@cliff-pi",
    workspace: "mosaico",
    title: "Add session status chrome to pi-mosaico",
    unhosted: true,
    headless: false,
  })
})

test("renders handle, workspace, title, and delivery", () => {
  assert.equal(
    renderSessionStatus({
      name: "@cliff-pi",
      workspace: "mosaico",
      title: "Add session status chrome to pi-mosaico",
      unhosted: true,
      headless: false,
    }),
    "@cliff-pi #mosaico [Add session status chrome to pi-mosaico] unhosted",
  )
})

test("omits hosted delivery and empty title", () => {
  assert.equal(
    renderSessionStatus({
      name: "@quill-pi",
      workspace: "mosaico",
      title: "",
      unhosted: false,
      headless: false,
    }),
    "@quill-pi #mosaico",
  )
})

test("unescapes quoted titles", () => {
  const status = parseSessionStatus(
    `<self name="@coder" workspace="root" title="A &amp; B &quot;draft&quot;" headless="off" />`,
  )
  assert.equal(status?.title, `A & B "draft"`)
})
