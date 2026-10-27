const CONTROL_CHARS = /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/g
const ANSI = /\u001b\[[0-?]*[ -/]*[@-~]/g

type RecordValue = Record<string, unknown>

export interface PanelSnapshot {
  running: boolean
  session: { name: string; state: string; title: string; statusAge: string }
  channels: Array<{ path: string; about: string }>
  interactions: Array<{ name: string; channel: string; age: string; message: string }>
  error: string
  updatedAt: number
}

export interface PanelSnapshotInput {
  running?: unknown
  overview?: unknown
  error?: unknown
}

export function cleanText(value: unknown, fallback = ""): string {
  if (typeof value !== "string") return fallback
  return value.replace(ANSI, "").replace(CONTROL_CHARS, "").trim()
}

function record(value: unknown): RecordValue | undefined {
  return value && typeof value === "object" ? value as RecordValue : undefined
}

export function panelSnapshot(input: PanelSnapshotInput): PanelSnapshot {
  const overview = record(input.overview)
  const channels = (Array.isArray(overview?.channels) ? overview.channels : [])
    .map(record)
    .map(channel => ({ path: cleanText(channel?.path), about: cleanText(channel?.about) }))
    .filter(channel => channel.path)
  const interactions = (Array.isArray(overview?.interactions) ? overview.interactions : [])
    .map(record)
    .map(interaction => ({
      name: cleanText(interaction?.name),
      channel: cleanText(interaction?.channel),
      age: cleanText(interaction?.age),
      message: cleanText(interaction?.message),
    }))
    .filter(interaction => interaction.name && interaction.channel && interaction.message)
  return {
    running: Boolean(input.running),
    session: {
      name: cleanText(overview?.name),
      state: cleanText(overview?.state, "unknown"),
      title: cleanText(overview?.title),
      statusAge: cleanText(overview?.status_age),
    },
    channels,
    interactions,
    error: cleanText(input.error),
    updatedAt: Date.now(),
  }
}
