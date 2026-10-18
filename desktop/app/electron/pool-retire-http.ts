interface PoolBackend {
  /** The backend kind it was spawned as: 'factr' is the Rust engine, which has no retirement door. */
  kind?: string
  process?: unknown
  port?: number | null
  token?: string | null
}

/** The permit an idle Rust engine is retired under (it has no backend-issued token to hold). */
const ENGINE_PERMIT = 'engine-idle'

/** The engine's own answer to `GET /api/sessions` (rows carry `is_active` while a turn runs). */
export async function engineHasNoRunningTurn(requestJson: RequestJson, entry: PoolBackend): Promise<boolean> {
  if (!entry.process || !entry.port || !entry.token) {
    return false
  }

  try {
    const reply = (await requestJson(
      `http://127.0.0.1:${entry.port}/api/sessions?limit=1000&include_archived=true`,
      entry.token,
      { method: 'GET', body: {}, timeoutMs: 3000 }
    )) as null | { sessions?: { is_active?: boolean }[] }

    // Hidden turns (a Bot Chat delivery, a cron run, a side agent) count too: they are archived rows of the same list.
    return Array.isArray(reply?.sessions) && !reply.sessions.some(row => row?.is_active === true)
  } catch {
    return false // no answer is no proof of idleness
  }
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

      // The Rust engine does not serve the retirement door (the Python backend does): a foreground open that needs a
      // slot would wait for one forever. Ask the engine itself whether any turn runs, and retire it only if none does.
      return reply === null && entry.kind === 'factr' && (await engineHasNoRunningTurn(requestJson, entry))
        ? ENGINE_PERMIT
        : null
    },
    commit: async (_key: string, entry: PoolBackend, token: string): Promise<boolean> =>
      token === ENGINE_PERMIT ? engineHasNoRunningTurn(requestJson, entry) : (await request(entry, 'commit', token))?.ok === true,
    cancel: async (_key: string, entry: PoolBackend, token: string): Promise<void> => {
      if (token !== ENGINE_PERMIT) {
        await request(entry, 'cancel', token)
      }
    }
  }
}
