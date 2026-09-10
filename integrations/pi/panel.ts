import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent"
import { readFile } from "node:fs/promises"
import { homedir } from "node:os"
import { join, resolve } from "node:path"
import { pathToFileURL } from "node:url"
import { caller, daemonCall } from "./protocol.ts"
import { panelSnapshot, type PanelSnapshot } from "./panel-model.ts"

const POLL_MS = 5_000
const PANEL_PACKAGE = "pi-panels"

type Sidebar = { publish(state: PanelSnapshot): void; dispose(): void }
type DefineSidebar = (pi: ExtensionAPI, definition: { id: string; title: string; view: URL }, options: { onMountChange(mounted: boolean): void }) => Sidebar

export async function installMosaicoPanel(pi: ExtensionAPI): Promise<void> {
  const defineSidebar = await loadDefineSidebar()
  if (!defineSidebar) return

  let stopped = true
  let context: ExtensionContext | undefined
  let timer: ReturnType<typeof setInterval> | undefined
  const sidebar = defineSidebar(pi, {
    id: "mosaico",
    title: "mosaico",
    view: new URL("./panel-view.ts", import.meta.url),
  }, { onMountChange: mounted => mounted ? start() : stop() })

  async function refresh(): Promise<void> {
    if (!context) return
    try {
      const overview = await daemonCall(
        "my_session_overview",
        caller(context),
        AbortSignal.timeout(2_500),
      )
      sidebar.publish(panelSnapshot({ running: true, overview }))
    } catch (error) {
      sidebar.publish(panelSnapshot({
        running: false,
        error: error instanceof Error ? error.message : "Mosaico daemon is unavailable.",
      }))
    }
  }

  function start(): void {
    if (!stopped) return
    stopped = false
    void refresh()
    timer = setInterval(() => void refresh(), POLL_MS)
  }

  function stop(): void {
    stopped = true
    if (timer) clearInterval(timer)
    timer = undefined
  }

  pi.on("session_start", (_event, ctx) => {
    context = ctx
    if (!stopped) void refresh()
  })
  pi.on("session_shutdown", () => {
    stop()
    sidebar.dispose()
  })
}

async function loadDefineSidebar(): Promise<DefineSidebar | undefined> {
  for (const module of ["pi-panels/sidebar", await configuredPanelModule()].filter((value): value is string => Boolean(value))) {
    try {
      const loaded = await import(module) as { defineSidebar?: unknown }
      if (typeof loaded.defineSidebar === "function") return loaded.defineSidebar as DefineSidebar
    } catch {
      // Pi Panels is optional; Mosaico's coordination extension remains usable without a sidebar host.
    }
  }
  return undefined
}

export async function configuredPanelModule(
  agentDir = process.env.PI_CODING_AGENT_DIR || join(homedir(), ".pi", "agent"),
): Promise<string | undefined> {
  try {
    const settings = JSON.parse(await readFile(join(agentDir, "settings.json"), "utf8")) as { packages?: unknown }
    for (const source of Array.isArray(settings.packages) ? settings.packages : []) {
      if (typeof source !== "string") continue
      const root = source === `npm:${PANEL_PACKAGE}`
        ? join(agentDir, "npm", "node_modules", PANEL_PACKAGE)
        : resolve(agentDir, source.replace(/^file:/, ""))
      try {
        const manifest = JSON.parse(await readFile(join(root, "package.json"), "utf8")) as {
          name?: unknown
          exports?: Record<string, unknown>
        }
        const sidebar = manifest.exports?.["./sidebar"]
        if (manifest.name === PANEL_PACKAGE && typeof sidebar === "string") {
          return pathToFileURL(resolve(root, sidebar)).href
        }
      } catch {
        continue
      }
    }
  } catch {
    return undefined
  }
  return undefined
}
