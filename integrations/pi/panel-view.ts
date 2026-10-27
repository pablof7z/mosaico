import { Spacer, Text, VStack } from "@earendil-works/pi-tui"
import type { PanelSnapshot } from "./panel-model.ts"

interface ViewContext {
  getState(): PanelSnapshot | undefined
  theme: { fg(name: string, value: string): string; bold(value: string): string }
}

function linesFor(state: PanelSnapshot | undefined, theme: ViewContext["theme"]): Array<Text | Spacer> {
  if (!state) return [new Text(theme.fg("dim", "Loading Mosaico…"), 0, 0)]
  if (!state.running) {
    const rows = [new Text(`${theme.bold("Mosaico")}  ${theme.fg("error", "○ unavailable")}`, 0, 0)]
    if (state.error) rows.push(new Text(theme.fg("dim", state.error), 0, 0))
    return rows
  }
  const rows: Array<Text | Spacer> = []
  if (state.session.name) {
    const broken = ["offline", "error", "failed"].includes(state.session.state)
    const suffix = broken ? theme.fg("error", ` · ${state.session.state}`) : ""
    rows.push(new Text(`${theme.fg("text", `@${state.session.name}`)}${suffix}`, 0, 0))
  }
  if (state.error) rows.push(new Text(theme.fg("error", state.error), 0, 0))
  if (state.session.title) {
    separate(rows)
    rows.push(new Text(theme.fg("dim", state.session.title), 0, 0))
    if (state.session.statusAge) rows.push(new Text(theme.fg("dim", state.session.statusAge), 0, 0))
  }
  if (state.channels.length) {
    separate(rows)
    const paths = new Set(state.channels.map(channel => channel.path))
    for (const channel of state.channels.slice(0, 6)) {
      const label = channelLabel(channel.path, paths)
      const about = channel.about ? `  ${theme.fg("dim", channel.about)}` : ""
      rows.push(new Text(`${theme.fg("text", label)}${about}`, 0, 0))
    }
  }
  if (state.interactions.length) {
    separate(rows)
    for (const interaction of state.interactions.slice(0, 6)) {
      const age = interaction.age ? ` ${theme.fg("dim", interaction.age)}` : ""
      rows.push(new Text(`${theme.fg("text", `@${interaction.name} on ${interaction.channel}`)}${age}`, 0, 0))
      rows.push(new Text(theme.fg("dim", interaction.message), 0, 0))
    }
  }
  return rows
}

function separate(rows: Array<Text | Spacer>): void {
  if (rows.length) rows.push(new Spacer())
}

function channelLabel(path: string, paths: Set<string>): string {
  const parts = path.slice(1).split("/")
  if (parts.length === 1) return path
  const parent = `#${parts.slice(0, -1).join("/")}`
  if (!paths.has(parent)) return path
  return `${"  ".repeat(parts.length - 2)}|- #${parts.at(-1)}`
}

export default function createSidebarView(ctx: ViewContext) {
  return { invalidate() {}, render(width: number) { return new VStack(linesFor(ctx.getState(), ctx.theme)).render(width) } }
}
