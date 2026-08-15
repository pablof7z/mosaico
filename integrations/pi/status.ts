import type { ExtensionContext } from "@earendil-works/pi-coding-agent"
import { execute } from "./protocol.ts"

const STATUS_KEY = "mosaico"
const TITLE_MAX = 48

export interface SessionStatus {
  name: string
  workspace: string
  title: string
  unhosted: boolean
  headless: boolean
}

export function parseSessionStatus(fabric: string): SessionStatus | undefined {
  const match = fabric.match(/<self\b([^>]*)\/>/)
  if (!match) return undefined
  const attrs = match[1]
  const name = attr(attrs, "name")
  if (!name) return undefined
  return {
    name,
    workspace: attr(attrs, "workspace"),
    title: attr(attrs, "title"),
    unhosted: attr(attrs, "unhosted") === "true",
    headless: attr(attrs, "headless") === "on",
  }
}

export function renderSessionStatus(status: SessionStatus): string {
  const parts = [status.name]
  if (status.workspace) parts.push(`#${status.workspace}`)
  if (status.title) parts.push(`[${truncate(status.title, TITLE_MAX)}]`)
  if (status.unhosted) parts.push("unhosted")
  else if (status.headless) parts.push("headless")
  return parts.join(" ")
}

function attr(source: string, name: string): string {
  const match = source.match(new RegExp(`\\b${name}="([^"]*)"`))
  return match ? unescape(match[1]) : ""
}

function unescape(value: string): string {
  return value
    .replaceAll("&quot;", '"')
    .replaceAll("&apos;", "'")
    .replaceAll("&lt;", "<")
    .replaceAll("&gt;", ">")
    .replaceAll("&amp;", "&")
}

function truncate(value: string, max: number): string {
  const chars = [...value]
  if (chars.length <= max) return value
  return `${chars.slice(0, max - 1).join("").trimEnd()}…`
}

function canPaint(ctx: ExtensionContext): boolean {
  return ctx.mode === "tui" && typeof ctx.ui?.setStatus === "function"
}

export async function paintSessionStatus(
  ctx: ExtensionContext,
  bin: string,
): Promise<void> {
  if (!canPaint(ctx)) return
  const result = await execute(bin, "mosaico_session", {}, AbortSignal.timeout(2_000), ctx)
  const fabric = typeof result.details?.fabric === "string" ? result.details.fabric : ""
  const status = !result.isError ? parseSessionStatus(fabric) : undefined
  ctx.ui.setStatus(STATUS_KEY, status ? renderSessionStatus(status) : undefined)
}

export function clearSessionStatus(ctx: ExtensionContext): void {
  if (!canPaint(ctx)) return
  ctx.ui.setStatus(STATUS_KEY, undefined)
}
