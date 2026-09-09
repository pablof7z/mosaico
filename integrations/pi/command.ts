import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent"
import { caller, daemonCall } from "./protocol.ts"

const COMMANDS = ["join"] as const
const USAGE = [
  "/mosaico — inspect this Pi session and the Mosaico fabric",
  "/mosaico join — choose a known channel to join",
  "/mosaico join <channel> — join a known channel",
].join("\n")

interface ChannelOption {
  path: string
  about: string
}

interface SessionSnapshot {
  running: boolean
  name: string
  state: string
  title: string
  channels: ChannelOption[]
  error: string
}

interface CommandDependencies {
  readSnapshot(ctx: ExtensionContext): Promise<SessionSnapshot>
  listChannels(ctx: ExtensionContext): Promise<ChannelOption[]>
  joinChannel(channel: string, ctx: ExtensionContext): Promise<string>
}

export function registerMosaicoCommand(
  pi: ExtensionAPI,
  dependencies: CommandDependencies = defaultDependencies,
): void {
  pi.registerCommand("mosaico", {
    description: "inspect this session or join a Mosaico channel",
    getArgumentCompletions: prefix => {
      const query = prefix.trim().toLowerCase()
      const matches = COMMANDS
        .filter(command => command.startsWith(query))
        .map(command => ({ value: command, label: command }))
      return matches.length ? matches : null
    },
    handler: async (args, ctx) => {
      const input = args.trim()
      try {
        if (!input) {
          ctx.ui.notify(renderOverview(await dependencies.readSnapshot(ctx)), "info")
          return
        }
        const match = /^join(?:\s+(\S+))?$/.exec(input)
        if (!match) {
          ctx.ui.notify(USAGE, "error")
          return
        }
        const channel = match[1] || await selectChannel(ctx, dependencies)
        if (!channel) return
        ctx.ui.notify(await dependencies.joinChannel(channel, ctx), "info")
      } catch (error) {
        const message = error instanceof Error ? error.message : "Mosaico is unavailable."
        ctx.ui.notify(`Mosaico unavailable: ${message}`, "error")
      }
    },
  })
}

export function renderOverview(snapshot: SessionSnapshot): string {
  const lines = [`Mosaico ${snapshot.running ? "running" : "stopped"}`]
  if (snapshot.error) lines.push("", snapshot.error)
  if (snapshot.running) {
    lines.push(`@${snapshot.name || "unknown"} · ${snapshot.state}`)
    lines.push(`Working on: ${snapshot.title || "No public status"}`)
    lines.push(`Channels: ${snapshot.channels.map(channel => channel.path).join(", ") || "none"}`)
  }
  return lines.join("\n")
}

async function selectChannel(
  ctx: ExtensionContext,
  dependencies: CommandDependencies,
): Promise<string | undefined> {
  const [snapshot, known] = await Promise.all([
    dependencies.readSnapshot(ctx),
    dependencies.listChannels(ctx),
  ])
  const active = new Set(snapshot.channels.map(channel => channel.path))
  const available = known.filter(channel => !active.has(channel.path))
  if (!available.length) {
    ctx.ui.notify("No known channels available to join.", "info")
    return undefined
  }
  const labels = available.map(channel => {
    const about = channel.about.replace(/\s+/g, " ").trim()
    return about ? `${channel.path} — ${about}` : channel.path
  })
  const selected = await ctx.ui.select("Join a Mosaico channel", labels)
  return selected === undefined ? undefined : available[labels.indexOf(selected)]?.path
}

async function readSnapshot(ctx: ExtensionContext): Promise<SessionSnapshot> {
  try {
    const overview = await daemonCall("my_session_overview", caller(ctx), AbortSignal.timeout(2_500))
    const value = record(overview)
    const channels = (Array.isArray(value?.channels) ? value.channels : [])
      .map(record)
      .map(channel => ({ path: text(channel?.path), about: text(channel?.about) }))
      .filter(channel => channel.path)
    return {
      running: true,
      name: text(value?.name),
      state: text(value?.state) || "unknown",
      title: text(value?.title),
      channels,
      error: "",
    }
  } catch (error) {
    return {
      running: false,
      name: "",
      state: "unknown",
      title: "",
      channels: [],
      error: error instanceof Error ? error.message : "Mosaico daemon is unavailable.",
    }
  }
}

function record(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === "object" ? value as Record<string, unknown> : undefined
}

function text(value: unknown): string {
  return typeof value === "string" ? value.replace(/[\u0000-\u001f\u007f]/g, "").trim() : ""
}

async function listChannels(ctx: ExtensionContext): Promise<ChannelOption[]> {
  const result = await daemonCall(
    "channel_list",
    { ...caller(ctx), recursive: true },
    AbortSignal.timeout(2_500),
  )
  const sections = (result as { sections?: unknown } | null)?.sections
  if (!Array.isArray(sections)) throw new Error("Channel list returned an invalid result.")
  const channels: ChannelOption[] = []
  const seen = new Set<string>()
  for (const section of sections) {
    const entries = (section as { channels?: unknown })?.channels
    if (Array.isArray(entries)) collectChannels(entries, channels, seen)
  }
  return channels
}

function collectChannels(entries: unknown[], channels: ChannelOption[], seen: Set<string>): void {
  for (const value of entries) {
    const entry = value as { path?: unknown; about?: unknown; children?: unknown }
    if (typeof entry.path === "string" && !seen.has(entry.path)) {
      seen.add(entry.path)
      channels.push({ path: entry.path, about: typeof entry.about === "string" ? entry.about : "" })
    }
    if (Array.isArray(entry.children)) collectChannels(entry.children, channels, seen)
  }
}

async function joinChannel(channel: string, ctx: ExtensionContext): Promise<string> {
  const result = await daemonCall(
    "channel_join",
    { ...caller(ctx), channel },
    AbortSignal.timeout(10_000),
  )
  const joined = (result as { channel?: unknown } | null)?.channel
  if (typeof joined !== "string") throw new Error("Channel join returned an invalid result.")
  return `Joined ${joined}.`
}

const defaultDependencies: CommandDependencies = { readSnapshot, listChannels, joinChannel }
