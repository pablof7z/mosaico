import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent"
import { DaemonConnection, caller, isHostedPi } from "./protocol.ts"

const DELIVERY_TYPE = "mosaico.delivery"
const DELIVERY_TIMEOUT_MS = 40_000

interface Delivery {
  kind: "delivery"
  lease_id: string
  message: {
    custom_type: string
    content: string
    display: boolean
    details?: Record<string, unknown>
  }
}

interface PendingDelivery {
  resolve: () => void
  reject: (error: Error) => void
}

export class DeliveryPump {
  private running = false
  private connection?: DaemonConnection
  private pending = new Map<string, PendingDelivery>()
  private readonly pi: ExtensionAPI
  private readonly ctx: ExtensionContext

  constructor(pi: ExtensionAPI, ctx: ExtensionContext) {
    this.pi = pi
    this.ctx = ctx
  }

  start(): void {
    if (this.running || (isHostedPi() && process.env.MOSAICO_TRANSPORT === "pi-rpc")) return
    this.running = true
    void this.run()
  }

  stop(): void {
    this.running = false
    this.connection?.close()
    this.connection = undefined
    for (const pending of this.pending.values()) {
      pending.reject(new Error("Pi Mosaico delivery pump stopped."))
    }
    this.pending.clear()
  }

  onMessageStart(event: unknown): void {
    const message = (event as { message?: Record<string, unknown> }).message
    if (message?.role !== "custom" || message.customType !== DELIVERY_TYPE) return
    const details = message.details as Record<string, unknown> | undefined
    const leaseId = typeof details?.lease_id === "string" ? details.lease_id : ""
    if (leaseId) void this.acknowledge(leaseId, true)
  }

  private async run(): Promise<void> {
    while (this.running) {
      try {
        const connection = await DaemonConnection.open()
        this.connection = connection
        while (this.running && this.connection === connection) {
          const result = await connection.call("session_delivery_wait", {
            ...caller(this.ctx),
            timeout_secs: 60,
          }) as { kind?: string }
          if (result.kind === "delivery") await this.deliver(result as Delivery)
        }
      } catch {
        if (this.running) await pause(1_000)
      } finally {
        this.connection?.close()
        this.connection = undefined
      }
    }
  }

  private async deliver(delivery: Delivery): Promise<void> {
    await new Promise<void>((resolve, reject) => {
      const timeout = setTimeout(() => {
        void this.acknowledge(delivery.lease_id, false)
        reject(new Error("Pi did not accept the Mosaico delivery in time."))
      }, DELIVERY_TIMEOUT_MS)
      this.pending.set(delivery.lease_id, {
        resolve: () => {
          clearTimeout(timeout)
          resolve()
        },
        reject: error => {
          clearTimeout(timeout)
          reject(error)
        },
      })
      this.pi.sendMessage({
        customType: delivery.message.custom_type,
        content: delivery.message.content,
        display: delivery.message.display,
        details: { ...delivery.message.details, lease_id: delivery.lease_id },
      }, { triggerTurn: true, deliverAs: "steer" })
    })
  }

  private async acknowledge(leaseId: string, accepted: boolean): Promise<void> {
    const pending = this.pending.get(leaseId)
    if (!pending || !this.connection) return
    this.pending.delete(leaseId)
    try {
      await this.connection.call("session_delivery_ack", {
        ...caller(this.ctx),
        lease_id: leaseId,
        accepted,
      })
      if (accepted) pending.resolve()
      else pending.reject(new Error("Pi declined the Mosaico delivery."))
    } catch (error) {
      pending.reject(error instanceof Error ? error : new Error(String(error)))
    }
  }
}

function pause(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms))
}
