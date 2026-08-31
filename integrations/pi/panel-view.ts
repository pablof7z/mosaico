import { Text, VStack } from "@earendil-works/pi-tui"
import type { PanelSnapshot } from "./panel-model.ts"

interface ViewContext {
  getState(): PanelSnapshot | undefined
  theme: { fg(name: string, value: string): string; bold(value: string): string }
}

function linesFor(state: PanelSnapshot | undefined, theme: ViewContext["theme"]): Text[] {
  if (!state) return [new Text(theme.fg("dim", "Loading Mosaico…"), 0, 0)]
  const rows = [new Text(`${theme.bold("Mosaico")}  ${theme.fg(state.running ? "success" : "error", state.running ? "● running" : "○ stopped")}`, 0, 0)]
  if (state.error) rows.push(new Text(theme.fg("dim", state.error), 0, 0))
  rows.push(new Text(theme.fg("accent", `Sessions  ${state.sessions.length}`), 0, 0))
  for (const session of state.sessions.slice(0, 6)) rows.push(new Text(`  ${theme.fg("text", session.agent)} ${theme.fg("dim", `· ${session.harness} · ${session.channel}`)}`, 0, 0))
  rows.push(new Text(theme.fg("accent", `Relays  ${state.relays.length}`), 0, 0))
  for (const relay of state.relays.slice(0, 4)) rows.push(new Text(`  ${theme.fg(state.running ? "success" : "dim", "●")} ${theme.fg("dim", `${relay.label} · ${state.running ? "daemon active" : "configured"}`)}`, 0, 0))
  rows.push(new Text(theme.fg("accent", `Channels  ${state.channels.length}`), 0, 0))
  for (const channel of state.channels.slice(0, 4)) rows.push(new Text(`  ${theme.fg("text", channel.path)}${channel.lastActivity ? theme.fg("dim", ` · ${channel.lastActivity}`) : ""}`, 0, 0))
  rows.push(new Text(theme.fg("accent", "Recent activity"), 0, 0))
  for (const activity of state.activity.slice(-5)) rows.push(new Text(`  ${theme.fg("dim", activity.channel ? `${activity.channel} · ` : "")}${activity.text}`, 0, 0))
  return rows
}

export default function createSidebarView(ctx: ViewContext) {
  return { invalidate() {}, render(width: number) { return new VStack(linesFor(ctx.getState(), ctx.theme)).render(width) } }
}
