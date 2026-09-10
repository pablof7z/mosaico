import type { ExtensionAPI } from "@earendil-works/pi-coding-agent"
import { execute, readChannel } from "./protocol.ts"

type Schema = Parameters<ExtensionAPI["registerTool"]>[0]["parameters"]

interface ToolSpec {
  name: string
  label: string
  description: string
  parameters: Schema
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
/** Unix timestamp (integer) or duration string (e.g. 2h, 30m, 10s, 7d). */
const time = (description: string) =>
  ({ anyOf: [{ type: "string" }, { type: "integer" }], description }) as Schema

function object(properties: Record<string, Schema>, required: string[] = []) {
  return {
    type: "object",
    properties,
    required,
    additionalProperties: false,
  } as Schema
}

const AGENT_SLUG_DESC = "Agent address as slug@backend, e.g. worker-claude@mosaico-local-e2e."
const TIME_DESC =
  "Unix timestamp or duration string such as 2h, 30m, 10s, or 7d. Durations subtract from current time."

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
      seconds: {
        type: "integer",
        minimum: 1,
        description: "Maximum seconds to wait for the next matching message.",
      } as Schema,
      channels: {
        type: "array",
        items: channel,
        description: "Channel paths to scope the wait. Defaults to all joined channels.",
      } as Schema,
      from: {
        type: "array",
        items: { type: "string" },
        description:
          "Agent identifiers (slug, name, or slug@backend) to filter by sender. Matched against channel members.",
      } as Schema,
    }, ["seconds"]),
  },
  {
    name: "mosaico_channel_list",
    label: "Mosaico Channel List",
    description: "List the agent-visible workspace and channel forest.",
    parameters: object({
      workspace: {
        type: "string",
        description: "Workspace name to scope the listing. At most one of workspace, all, or recursive may be set.",
      } as Schema,
      all: {
        type: "boolean",
        description: "List channels across every workspace. At most one of workspace, all, or recursive may be set.",
      } as Schema,
      recursive: {
        type: "boolean",
        description: "List channels recursively within the current workspace. At most one of workspace, all, or recursive may be set.",
      } as Schema,
    }),
  },
  {
    name: "mosaico_channel_read",
    label: "Mosaico Channel Read",
    description: "Read recent messages, or one complete message by id, from a joined channel.",
    parameters: object({
      channel,
      id: {
        type: "string",
        description:
          "6-character message id to read in full. Mutually exclusive with since and limit.",
      } as Schema,
      since: time(`Time lower bound. ${TIME_DESC}`),
      limit: {
        type: "integer",
        minimum: 1,
        description: "Maximum number of recent messages to return.",
      } as Schema,
    }),
  },
  {
    name: "mosaico_channel_search",
    label: "Mosaico Channel Search",
    description: "Search observed messages by author, recipient, text, time, or channel subtree.",
    parameters: object({
      from: {
        type: "array",
        items: { type: "string" },
        description: "Author identifiers (slug, name, or slug@backend) to filter by sender.",
      } as Schema,
      to: strings,
      contains: {
        type: "array",
        items: { type: "string" },
        description: "Words or phrases that must all appear in the message body.",
      } as Schema,
      channels: {
        type: "array",
        items: channel,
        description: "Channel paths to restrict the search.",
      } as Schema,
      since: time(`Lower time bound. ${TIME_DESC}`),
      until: time(`Upper time bound. ${TIME_DESC}`),
      limit: {
        type: "integer",
        minimum: 1,
        description: "Maximum number of messages to return.",
      } as Schema,
      cursor: {
        type: "string",
        description:
          "Opaque continuation token from a previous search result. Must be used alone; it already encodes the full query.",
      } as Schema,
    }),
  },
  {
    name: "mosaico_send",
    label: "Mosaico Send",
    description: "Send a new channel message, optionally tagging agents or awaiting a correlated reply.",
    parameters: object({
      message,
      channel,
      tags: {
        type: "array",
        items: { type: "string" },
        description:
          "Agent slugs or slug@backend references to tag; delivers the message to their directed inboxes.",
      } as Schema,
      attachments,
      force: {
        type: "boolean",
        description:
          "Bypass the mention-guard: publish the message even when the body contains @mention-like text and no tags are set. Use when the @ is literal, not an agent address.",
      } as Schema,
      seconds: {
        type: "integer",
        minimum: 1,
        description:
          "Seconds to wait for a correlated reply after sending. Returns a message-or-timeout outcome alongside the send result.",
      } as Schema,
    }, ["message"]),
  },
  {
    name: "mosaico_reply",
    label: "Mosaico Reply",
    description: "Reply substantively to one message in its original channel; react for a bare acknowledgement.",
    parameters: object({
      id: {
        type: "string",
        description: "6-character id of the message to reply to.",
      } as Schema,
      message,
      attachments,
    }, ["id", "message"]),
  },
  {
    name: "mosaico_react",
    label: "Mosaico React",
    description: "Acknowledge one message non-disruptively with an emoji instead of replying.",
    parameters: object({
      id: {
        type: "string",
        description: "6-character id of the message to react to.",
      } as Schema,
      emoji: {
        type: "string",
        description: "Unicode emoji to react with.",
      } as Schema,
    }, ["id", "emoji"]),
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
        items: { type: "string", description: AGENT_SLUG_DESC },
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
      target: { type: "string", description: AGENT_SLUG_DESC } as Schema,
      workspace: {
        type: "string",
        description: "Workspace to launch the agent into.",
      } as Schema,
      channels: {
        type: "array",
        items: channel,
        description: "Initial channels to assign to the dispatched session.",
      } as Schema,
      message,
    }, ["target", "workspace", "message"]),
  },
]

export function registerMosaicoTools(pi: ExtensionAPI) {
  for (const tool of tools) {
    pi.registerTool({
      ...tool,
      promptSnippet: tool.description,
      promptGuidelines: [
        "Use Mosaico tools for fabric coordination instead of shelling out to its CLI.",
        "Reply for substantive context; react for a bare acknowledgement.",
      ],
      execute: async (_id, params, signal, _update, ctx) =>
        tool.name === "mosaico_channel_read"
          ? readChannel(params as Record<string, unknown>, signal, ctx)
          : execute(tool.name, params as Record<string, unknown>, signal, ctx),
    })
  }
}
