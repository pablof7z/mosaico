import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const root = new URL("../", import.meta.url)

test("the npm package ships every local TypeScript dependency", async () => {
  const manifest = JSON.parse(await readFile(new URL("package.json", root), "utf8"))
  const shipped = new Set(manifest.files)
  const imports = /from\s+["']\.\/([^"']+\.ts)["']/g

  for (const name of [...shipped].filter(path => path.endsWith(".ts"))) {
    const source = await readFile(new URL(name, root), "utf8")
    for (const match of source.matchAll(imports)) {
      assert.ok(shipped.has(match[1]), `${name} imports unshipped ${match[1]}`)
    }
  }
})
