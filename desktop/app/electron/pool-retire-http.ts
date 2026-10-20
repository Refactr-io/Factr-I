interface PoolBackend {
  /** The backend kind it was spawned as. */
  kind?: string
  process?: unknown
  port?: number | null
  token?: string | null
}

interface RetirementReply {
  ok?: boolean
  idle?: boolean | null
  token?: string
}

type RequestJson = (
  url: string,
  token: string,
  options: { method: string; body: Record<string, string>; timeoutMs: number }
) => Promise<unknown>

/** Use the app's authenticated transport, never a descriptor's remote URL. */
export function createPoolRetirementClient(requestJson: RequestJson) {
  async function request(entry: PoolBackend, action: string, token?: string): Promise<RetirementReply | null> {
    if (!entry.process || !entry.port || !entry.token) {
      return null
    }

    try {
      return (await requestJson(`http://127.0.0.1:${entry.port}/api/health/retirement`, entry.token, {
        method: 'POST',
        body: { action, ...(token ? { token } : {}) },
        timeoutMs: 3000
      })) as RetirementReply | null
    } catch {
      // An older runtime, transport failure or unreadable reply grants no
      // authority. Prepared permits expire; committed ones remain recoverable
      // through idempotent prepare/commit on this exact backend generation.
      return null
    }
  }

  return {
    prepare: async (_key: string, entry: PoolBackend): Promise<string | null> => {
      const reply = await request(entry, 'prepare')

      if (reply?.ok === true && reply.idle === true && typeof reply.token === 'string' && reply.token) {
        return reply.token
      }

      return null
    },
    commit: async (_key: string, entry: PoolBackend, token: string): Promise<boolean> => {
      let reply = await request(entry, 'commit', token)
      if (reply?.ok !== true) { return false }
      const deadline = Date.now() + 60_000
      while (reply.idle === false && Date.now() < deadline) {
        await new Promise(resolve => setTimeout(resolve, 100))
        reply = await request(entry, 'status', token)
        if (reply?.ok !== true) { return false }
      }
      if (reply.idle === false) {
        reply = await request(entry, 'cancel_turns', token)
        if (reply?.ok !== true) { return false }
        for (let i = 0; i < 50 && reply.idle === false; i++) {
          await new Promise(resolve => setTimeout(resolve, 100))
          reply = await request(entry, 'status', token)
          if (reply?.ok !== true) { return false }
        }
      }
      return reply.idle !== false
    },
    cancel: async (_key: string, entry: PoolBackend, token: string): Promise<void> => {
      await request(entry, 'cancel', token)
    }
  }
}
