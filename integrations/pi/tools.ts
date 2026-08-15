import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent"
import { execFile } from "node:child_process"

type Params = Record<string, unknown>
type Details = Record<string, unknown> | null
type Schema = Parameters<ExtensionAPI["registerTool"]>[0]["parameters"]

interface ToolSpec {
  name: string
  label: string
  description: string
  parameters: Schema
}

interface ProtocolResult {
  content: Array<{ type: "text"; text: string }>
  details: Details
  isError: boolean
}

const channel = {
  type: "string",
  description: "Full channel path such as #workspace/task.",
} as Schema
const message = { type: "string", description: "Message body." } as Schema
const attachments = {
  type: "array",
  items: {
    type: "string",
    description: "Local FILE or LABEL=FILE attachment specification.",
  },
} as Schema
const strings = { type: "array", items: { type: "string" } } as Schema
const string = { type: "string" } as Schema
const integer = (minimum?: number) => ({ type: "integer", minimum }) as Schema
const time = {
  anyOf: [{ type: "string" }, { type: "integer" }],
} as Schema

function object(properties: Record<string, Schema>, required: string[] = []) {
  return {
    type: "object",
    properties,
    required,
    additionalProperties: false,
  } as Schema
}

const tools: ToolSpec[] = [
  {
    name: "mosaico_session",
    label: "Mosaico Session",
    description: "Read this agent's full Mosaico identity, memberships, peers, and awareness.",
    parameters: object({}),
  },
  {
    name: "mosaico_wait",
    label: "Mosaico Wait",
    description: "Wait once for the next matching fabric message without polling.",
    parameters: object({
      timeout_seconds: integer(1),
      channels: { type: "array", items: channel } as Schema,
      from: string,
    }, ["timeout_seconds"]),
  },
  {
    name: "mosaico_channel_list",
    label: "Mosaico Channel List",
    description: "List the agent-visible workspace and channel forest.",
    parameters: object({
      workspace: string,
      all: { type: "boolean" } as Schema,
      recursive: { type: "boolean" } as Schema,
    }),
  },
  {
    name: "mosaico_channel_read",
    label: "Mosaico Channel Read",
    description: "Read recent messages, or one complete message by id, from a joined channel.",
    parameters: object({
      channel,
      id: string,
      since: time,
      limit: integer(1),
    }),
  },
  {
    name: "mosaico_channel_search",
    label: "Mosaico Channel Search",
    description: "Search observed messages by author, recipient, text, time, or channel subtree.",
    parameters: object({
      from: strings,
      to: strings,
      contains: strings,
      channels: { type: "array", items: channel } as Schema,
      since: time,
      until: time,
      limit: integer(1),
      cursor: string,
    }),
  },
  {
    name: "mosaico_send",
    label: "Mosaico Send",
    description: "Send a new channel message, optionally tagging agents or awaiting a correlated reply.",
    parameters: object({
      message,
      channel,
      tags: strings,
      attachments,
      force: { type: "boolean" } as Schema,
      wait_seconds: integer(1),
    }, ["message"]),
  },
  {
    name: "mosaico_reply",
    label: "Mosaico Reply",
    description: "Reply substantively to one message in its original channel; react for a bare acknowledgement.",
    parameters: object({
      message_id: string,
      message,
      attachments,
    }, ["message_id", "message"]),
  },
  {
    name: "mosaico_react",
    label: "Mosaico React",
    description: "Acknowledge one message non-disruptively with an emoji instead of replying.",
    parameters: object({
      message_id: string,
      emoji: string,
    }, ["message_id", "emoji"]),
  },
  {
    name: "mosaico_channel_create",
    label: "Mosaico Channel Create",
    description: "Create and additively join one new leaf task channel whose parent already exists.",
    parameters: object({
      channel,
      about: { type: "string", maxLength: 80 } as Schema,
      agents: {
        type: "array",
        items: { type: "string", description: "Agent target as slug@backend." },
      } as Schema,
    }, ["channel", "about"]),
  },
  {
    name: "mosaico_channel_join",
    label: "Mosaico Channel Join",
    description: "Join an existing channel for passive context and direct-mention delivery.",
    parameters: object({ channel }, ["channel"]),
  },
  {
    name: "mosaico_channel_leave",
    label: "Mosaico Channel Leave",
    description: "Stop listening to a passively joined channel.",
    parameters: object({ channel }, ["channel"]),
  },
  {
    name: "mosaico_dispatch",
    label: "Mosaico Dispatch",
    description: "Start a fabric agent session only when no existing session already owns the work.",
    parameters: object({
      target: { type: "string", description: "Agent or agent@backend target." } as Schema,
      workspace: string,
      channels: { type: "array", items: channel } as Schema,
      message,
    }, ["target", "workspace", "message"]),
  },
]

function protocolError(message: string): ProtocolResult {
  return {
    content: [{ type: "text", text: message }],
    details: { error: message },
    isError: true,
  }
}

function execute(
  bin: string,
  tool: string,
  params: Params,
  signal: AbortSignal | undefined,
  ctx: ExtensionContext,
): Promise<ProtocolResult> {
  return new Promise(resolve => {
    let settled = false
    const finish = (result: ProtocolResult) => {
      if (settled) return
      settled = true
      signal?.removeEventListener("abort", onAbort)
      resolve(result)
    }
    const child = execFile(bin, ["harness", "pi"], {
      cwd: ctx.cwd,
      maxBuffer: 8 * 1024 * 1024,
    }, (error, stdout, stderr) => {
      if (error) {
        finish(protocolError(
          stderr.trim() || `Mosaico ${tool} transport failed: ${error.message}`,
        ))
        return
      }
      try {
        const result = JSON.parse(stdout) as ProtocolResult
        if (!Array.isArray(result.content) || typeof result.isError !== "boolean") {
          throw new Error("response is not a Pi tool result")
        }
        finish(result)
      } catch (parseError) {
        const reason = parseError instanceof Error ? parseError.message : String(parseError)
        finish(protocolError(`Mosaico ${tool} returned invalid JSON: ${reason}`))
      }
    })
    const onAbort = () => {
      child.kill()
      finish(protocolError(`Mosaico ${tool} was aborted.`))
    }
    signal?.addEventListener("abort", onAbort, { once: true })
    if (signal?.aborted) onAbort()
    child.stdin?.end(JSON.stringify({
      version: 1,
      tool,
      arguments: params,
      session: {
        native_id: ctx.sessionManager.getSessionId(),
        cwd: ctx.cwd,
        pty_session: process.env.MOSAICO_PTY_SESSION || undefined,
      },
    }))
  })
}

export function registerMosaicoTools(pi: ExtensionAPI, bin: string) {
  for (const tool of tools) {
    pi.registerTool({
      ...tool,
      promptSnippet: tool.description,
      promptGuidelines: [
        "Use Mosaico tools for fabric coordination instead of shelling out to its CLI.",
        "Reply for substantive context; react for a bare acknowledgement.",
      ],
      execute: async (_id, params, signal, _update, ctx) =>
        execute(bin, tool.name, params as Params, signal, ctx),
    })
  }
}
