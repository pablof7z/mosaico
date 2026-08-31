const CONTROL_CHARS = /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/g
const ANSI = /\u001b\[[0-?]*[ -/]*[@-~]/g

type RecordValue = Record<string, unknown>

export interface PanelSnapshot {
  home: string
  running: boolean
  daemonVersion: string
  relays: Array<{ url: string; label: string }>
  sessions: Array<{ agent: string; harness: string; channel: string; state: string }>
  channels: Array<{ path: string; about: string; agents?: number; lastActivity: string }>
  activity: Array<{ category: string; channel: string; text: string }>
  error: string
  updatedAt: number
}

export interface PanelSnapshotInput {
  home?: unknown
  running?: unknown
  daemonVersion?: unknown
  relays?: unknown
  sessions?: unknown
  channels?: unknown
  activity?: unknown
  error?: unknown
}

export function cleanText(value: unknown, fallback = ""): string {
  if (typeof value !== "string") return fallback
  return value.replace(ANSI, "").replace(CONTROL_CHARS, "").trim()
}

function record(value: unknown): RecordValue | undefined {
  return value && typeof value === "object" ? value as RecordValue : undefined
}

function shortRelay(url: string): string {
  try {
    const parsed = new URL(url)
    return parsed.host + parsed.pathname.replace(/\/$/, "")
  } catch {
    return cleanText(url, "unknown relay")
  }
}

function channelNames(workspaces: unknown): string[] {
  const names: string[] = []
  for (const workspace of Array.isArray(workspaces) ? workspaces : []) {
    const channels = record(workspace)?.channels
    for (const channel of Array.isArray(channels) ? channels : []) {
      const item = record(channel)
      const name = cleanText(item?.name || item?.id)
      if (name && !names.includes(name)) names.push(name)
    }
  }
  return names
}

function sessionRows(value: unknown): PanelSnapshot["sessions"] {
  const sessions = record(value)?.sessions
  return (Array.isArray(sessions) ? sessions : [])
    .map(record)
    .filter((row): row is RecordValue => row !== undefined && row.running !== false)
    .map(row => ({
      agent: cleanText(row.agent || row.handle, "unknown agent"),
      harness: cleanText(row.harness || row.transport, "unknown harness"),
      channel: channelNames(row.workspaces).join(", ") || "no channel",
      state: cleanText(row.state, "unknown"),
    }))
}

function collectChannels(entries: unknown, output: PanelSnapshot["channels"]): void {
  for (const value of Array.isArray(entries) ? entries : []) {
    const entry = record(value)
    if (!entry) continue
    const path = cleanText(entry.path)
    if (path) output.push({
      path,
      about: cleanText(entry.about),
      agents: Number.isSafeInteger(entry.agents) ? entry.agents as number : undefined,
      lastActivity: cleanText(entry.last_activity),
    })
    collectChannels(entry.children, output)
  }
}

function channelRows(value: unknown): PanelSnapshot["channels"] {
  const output: PanelSnapshot["channels"] = []
  const sections = record(value)?.sections
  for (const section of Array.isArray(sections) ? sections : []) collectChannels(record(section)?.channels, output)
  return output
}

export function activityRow(event: unknown): PanelSnapshot["activity"][number] | undefined {
  const item = record(event)
  if (!item) return undefined
  const category = cleanText(item.category, "event")
  const channel = cleanText(item.channel)
  const text = category === "msg" ? `${cleanText(item.from, "agent")}  ${cleanText(item.body)}`
    : category === "turn" || category === "stat" || category === "sess" ? `${cleanText(item.agent, "agent")} ${cleanText(item.state || item.text)}`
      : category === "join" ? `${cleanText(item.agent, "agent")} joined`
        : category === "leave" ? `${cleanText(item.agent, "agent")} left`
        : category === "sync" ? `${cleanText(item.to, "agent")} ${cleanText(item.state)}`
          : category === "proj" ? cleanText(item.about, "channel updated")
            : cleanText(item.text || item.body, category)
  return { category, channel, text: cleanText(text, category) }
}

export function panelSnapshot(input: PanelSnapshotInput): PanelSnapshot {
  return {
    home: cleanText(input.home),
    running: Boolean(input.running),
    daemonVersion: cleanText(input.daemonVersion),
    relays: (Array.isArray(input.relays) ? input.relays : []).map(url => cleanText(url)).filter(Boolean).map(url => ({ url, label: shortRelay(url) })),
    sessions: sessionRows(input.sessions),
    channels: channelRows(input.channels),
    activity: (Array.isArray(input.activity) ? input.activity : []).map(activityRow).filter((row): row is PanelSnapshot["activity"][number] => row !== undefined).slice(-8),
    error: cleanText(input.error),
    updatedAt: Date.now(),
  }
}
