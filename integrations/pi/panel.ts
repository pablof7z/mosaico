import type { ExtensionAPI } from "@earendil-works/pi-coding-agent"
import { readFile } from "node:fs/promises"
import { homedir } from "node:os"
import { join, resolve } from "node:path"
import { pathToFileURL } from "node:url"
import { DaemonConnection, daemonCall, mosaicoHome } from "./protocol.ts"
import { activityRow, panelSnapshot, type PanelSnapshot, type PanelSnapshotInput } from "./panel-model.ts"

const POLL_MS = 5_000
const PANEL_PACKAGE = "pi-panels"

type Sidebar = { publish(state: PanelSnapshot): void; dispose(): void }
type DefineSidebar = (pi: ExtensionAPI, definition: { id: string; title: string; view: URL }, options: { onMountChange(mounted: boolean): void }) => Sidebar

export async function installMosaicoPanel(pi: ExtensionAPI): Promise<void> {
  const defineSidebar = await loadDefineSidebar()
  if (!defineSidebar) return

  let stopped = true
  let timer: ReturnType<typeof setInterval> | undefined
  let closeTail: (() => void) | undefined
  let activity: PanelSnapshot["activity"] = []
  let daemonVersion = ""
  const home = mosaicoHome()
  const sidebar = defineSidebar(pi, {
    id: "mosaico",
    title: "mosaico",
    view: new URL("./panel-view.ts", import.meta.url),
  }, { onMountChange: mounted => mounted ? start() : stop() })

  const publish = (patch: PanelSnapshotInput) => sidebar.publish(panelSnapshot({
    home,
    activity,
    daemonVersion,
    ...patch,
  }))

  async function refresh(): Promise<void> {
    const relays = await configuredRelays(home)
    try {
      const [ping, sessions, channels] = await Promise.all([
        daemonCall("ping", {}, AbortSignal.timeout(2_500)),
        daemonCall("operator_sessions", {}, AbortSignal.timeout(2_500)),
        daemonCall("channel_list", { all: true }, AbortSignal.timeout(2_500)),
      ])
      const version = ping && typeof ping === "object" ? (ping as { daemonVersion?: unknown }).daemonVersion : undefined
      if (typeof version === "string") daemonVersion = version
      publish({ running: true, relays, sessions, channels, error: "" })
      if (!closeTail) void connectTail()
    } catch (error) {
      publish({ running: false, relays, sessions: {}, channels: {}, error: error instanceof Error ? error.message : "Mosaico daemon is unavailable." })
    }
  }

  async function connectTail(): Promise<void> {
    if (stopped || closeTail) return
    try {
      const connection = await DaemonConnection.open(AbortSignal.timeout(2_500))
      if (stopped) return connection.close()
      closeTail = connection.subscribe("tail", { backfill: 8 }, event => {
        const row = activityRow(event)
        if (!row) return
        activity = [...activity, row].slice(-8)
        void refresh()
      }, () => {
        closeTail = undefined
        if (!stopped) setTimeout(() => void connectTail(), POLL_MS)
      })
    } catch {
      if (!stopped) setTimeout(() => void connectTail(), POLL_MS)
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
    closeTail?.()
    closeTail = undefined
  }

  pi.on("session_shutdown", () => {
    stop()
    sidebar.dispose()
  })
}

async function configuredRelays(home: string): Promise<string[]> {
  try {
    const config = JSON.parse(await readFile(join(home, "config.json"), "utf8")) as { relays?: unknown }
    return Array.isArray(config.relays) ? config.relays.filter((relay): relay is string => typeof relay === "string") : []
  } catch {
    return []
  }
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

async function configuredPanelModule(): Promise<string | undefined> {
  const agentDir = process.env.PI_CODING_AGENT_DIR || join(homedir(), ".pi", "agent")
  try {
    const settings = JSON.parse(await readFile(join(agentDir, "settings.json"), "utf8")) as { packages?: unknown }
    for (const source of Array.isArray(settings.packages) ? settings.packages : []) {
      if (typeof source !== "string") continue
      const root = source === `npm:${PANEL_PACKAGE}`
        ? join(agentDir, "npm", "node_modules", PANEL_PACKAGE)
        : resolve(agentDir, source.replace(/^file:/, ""))
      try {
        const manifest = JSON.parse(await readFile(join(root, "package.json"), "utf8")) as { name?: unknown }
        if (manifest.name === PANEL_PACKAGE) return pathToFileURL(join(root, "extensions", PANEL_PACKAGE, "src", "sidebar.ts")).href
      } catch {
        continue
      }
    }
  } catch {
    return undefined
  }
  return undefined
}
