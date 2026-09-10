import assert from "node:assert/strict"
import { mkdtemp, mkdir, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { pathToFileURL } from "node:url"
import { configuredPanelModule } from "../panel.ts"

test("resolves the current pi-panels ./sidebar package export", async () => {
  const agentDir = await mkdtemp(join(tmpdir(), "pi-mosaico-panels-"))
  const packageDir = join(agentDir, "npm", "node_modules", "pi-panels")
  await mkdir(join(packageDir, "src"), { recursive: true })
  await writeFile(join(agentDir, "settings.json"), JSON.stringify({ packages: ["npm:pi-panels"] }))
  await writeFile(join(packageDir, "package.json"), JSON.stringify({
    name: "pi-panels",
    exports: { "./sidebar": "./src/sidebar.ts" },
  }))

  const resolved = await configuredPanelModule(agentDir)
  assert.equal(resolved, pathToFileURL(join(packageDir, "src", "sidebar.ts")).href)
  assert.doesNotMatch(resolved, /extensions\/pi-panels/)
})
