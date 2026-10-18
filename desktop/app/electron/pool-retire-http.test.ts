import assert from 'node:assert/strict'

import { test } from 'vitest'

import { createPoolRetirementClient, engineHasNoRunningTurn } from './pool-retire-http'

const entry = (kind = 'factr') => ({ kind, port: 4242, process: {}, token: 't' })

/** A transport that answers the retirement door and the session list the way a Rust engine and a Python backend do. */
function transport(opts: { door?: 'engine' | 'python-idle' | 'python-busy'; sessions?: { is_active?: boolean }[] | 'down' }) {
  const calls: string[] = []

  const request = async (url: string, _token: string, options: { method: string; body: Record<string, string> }) => {
    calls.push(`${options.method} ${url.replace('http://127.0.0.1:4242', '')}${options.body.action ? ` ${options.body.action}` : ''}`)

    if (url.endsWith('/api/health/retirement')) {
      if (opts.door === 'python-idle') {
        return { ok: true, idle: true, token: options.body.action === 'prepare' ? 'py-token' : undefined }
      }

      if (opts.door === 'python-busy') {
        return { ok: true, idle: false }
      }

      throw new Error('404 not_supported_by_engine') // the engine's answer
    }

    if (opts.sessions === 'down') {
      throw new Error('connection refused')
    }

    return { sessions: opts.sessions ?? [] }
  }

  return { calls, request }
}

test('a Rust engine has no retirement door: it is retired when its own session list shows no running turn', async () => {
  const { calls, request } = transport({ door: 'engine', sessions: [{ is_active: false }, {}] })
  const client = createPoolRetirementClient(request)
  const token = await client.prepare('scout', entry())

  assert.equal(token, 'engine-idle')
  assert.equal(await client.commit('scout', entry(), token!), true)
  await client.cancel('scout', entry(), token!)
  assert.deepEqual(calls.filter(call => call.includes('/api/health/retirement')), ['POST /api/health/retirement prepare'])
  assert.ok(calls.some(call => call.startsWith('GET /api/sessions?')), 'it asked the engine, not the desktop flag')
})

test('a Rust engine with a running turn (a hidden Bot Chat or cron run counts) is never retired', async () => {
  const client = createPoolRetirementClient(transport({ door: 'engine', sessions: [{ is_active: false }, { is_active: true }] }).request)

  assert.equal(await client.prepare('scout', entry()), null)
})

test('no answer from the engine is no proof of idleness', async () => {
  const client = createPoolRetirementClient(transport({ door: 'engine', sessions: 'down' }).request)

  assert.equal(await client.prepare('scout', entry()), null)
  assert.equal(await engineHasNoRunningTurn(transport({ sessions: 'down' }).request, entry()), false)
})

test('an engine that turned busy between prepare and commit is not stopped', async () => {
  const state = { sessions: [] as { is_active?: boolean }[] }

  const request = async (url: string) => {
    if (url.endsWith('/api/health/retirement')) {
      throw new Error('404')
    }

    return { sessions: state.sessions }
  }

  const client = createPoolRetirementClient(request)
  const token = await client.prepare('scout', entry())

  assert.equal(token, 'engine-idle')
  state.sessions = [{ is_active: true }]
  assert.equal(await client.commit('scout', entry(), token!), false)
})

test('a Python backend keeps its own authority: its busy answer is final and its token is used', async () => {
  const busy = createPoolRetirementClient(transport({ door: 'python-busy', sessions: [] }).request)

  assert.equal(await busy.prepare('p', entry('python')), null)
  // Even an engine-kind entry whose door answered "busy" is not retired behind its back.
  assert.equal(await busy.prepare('p', entry('factr')), null)

  const idle = transport({ door: 'python-idle' })
  const client = createPoolRetirementClient(idle.request)

  assert.equal(await client.prepare('p', entry('python')), 'py-token')
  assert.equal(await client.commit('p', entry('python'), 'py-token'), true)
  assert.ok(!idle.calls.some(call => call.startsWith('GET')), 'a Python backend is never asked for sessions')
})

test('an entry with no process, port or token grants nothing', async () => {
  const client = createPoolRetirementClient(transport({ door: 'engine', sessions: [] }).request)

  assert.equal(await client.prepare('x', { kind: 'factr', port: 1, token: 't' }), null)
  assert.equal(await client.prepare('x', { kind: 'factr', process: {}, token: 't' }), null)
})

test('a foreground open with all slots busy evicts the least recently used IDLE engine, never a busy one', async () => {
  const { createPoolRetirer } = await import('./pool-retire')
  const { LocalBackendSpawnCoordinator } = await import('./pool-spawn-coordinator')

  // Three resident bot engines on a cap of 3: a (oldest) is mid-turn, b is idle, c is idle and newer.
  const coordinator = new LocalBackendSpawnCoordinator(3)
  const releases = new Map<string, () => void>()
  const turns: Record<string, boolean> = { a: true, b: false, c: false }

  const pool = new Map(
    ['a', 'b', 'c'].map((key, i) => [key, { kind: 'factr', lastActiveAt: 100 + i, port: 4000 + i, process: {}, token: 't' }])
  )

  for (const key of pool.keys()) {
    releases.set(key, await coordinator.request(key).acquired)
  }

  assert.equal(coordinator.activeCount, 3)

  const request = async (url: string) => {
    if (url.includes('/api/health/retirement')) {
      throw new Error('404 not_supported_by_engine')
    }

    const key = ['a', 'b', 'c'][Number(new URL(url).port) - 4000]

    return { sessions: [{ is_active: turns[key] }] }
  }

  const stopped: string[] = []
  const retiring: string[] = []

  const retirer = createPoolRetirer({
    coordinator,
    onRetiring: key => retiring.push(key),
    pool,
    stopBackend: async key => {
      stopped.push(key)
      pool.delete(key)
      releases.get(key)!()
    },
    ...createPoolRetirementClient(request)
  })

  // The foreground open of a fourth bot waits for a slot, which wakes the arbiter.
  const open = coordinator.request('d', { priority: 'foreground', timeoutMs: 30_000 })

  assert.equal(open.queued, true)

  const release = await Promise.race([open.acquired, new Promise<never>((_, reject) => setTimeout(() => reject(new Error('no slot freed')), 3000))])

  assert.deepEqual(stopped, ['b'], 'the oldest engine was mid-turn and is skipped: the LRU idle one goes')
  assert.deepEqual(retiring, ['b'])
  assert.ok(pool.has('a') && pool.has('c'), 'the busy engine and the newer idle one stay resident')
  release()
  retirer.dispose()
})

test('a foreground open evicts a recently used idle engine at once; everything else spares it for the keepalive window', async () => {
  const { createPoolRetirer, evictionFreshMs } = await import('./pool-retire')
  const { LocalBackendSpawnCoordinator } = await import('./pool-spawn-coordinator')

  assert.equal(evictionFreshMs('foreground', 240_000), 0)
  assert.equal(evictionFreshMs('background', 240_000), 240_000)
  assert.equal(evictionFreshMs(undefined, 240_000), 240_000)

  const build = () => {
    const pool = new Map(
      ['a', 'b', 'c'].map((key, i) => [key, { kind: 'factr', lastActiveAt: Date.now() - 1000 * (3 - i), port: 4000 + i, process: {}, token: 't' }])
    )

    const stopped: string[] = []

    const retirer = createPoolRetirer({
      coordinator: new LocalBackendSpawnCoordinator(3),
      pool,
      stopBackend: async key => {
        stopped.push(key)
        pool.delete(key)
      },
      ...createPoolRetirementClient(async (url: string) => {
        if (url.includes('/api/health/retirement')) {
          throw new Error('404')
        }

        return { sessions: [] }
      })
    })

    return { pool, retirer, stopped }
  }

  // All three were touched seconds ago (inside the keepalive window) and none runs a turn.
  const background = build()

  assert.deepEqual(await background.retirer.evictTo(2, evictionFreshMs('background', 240_000)), [])
  assert.deepEqual(background.stopped, [])

  const foreground = build()

  assert.deepEqual(await foreground.retirer.evictTo(2, evictionFreshMs('foreground', 240_000)), ['a'], 'the least recently used goes')
  assert.deepEqual(foreground.stopped, ['a'])
  assert.equal(foreground.pool.size, 2)

  // Two foreground opens in a row are serialised: the second sees the first one's room already made.
  const burst = build()
  const [first, second] = await Promise.all([burst.retirer.evictTo(2, 0), burst.retirer.evictTo(2, 0)])

  assert.deepEqual([...first, ...second], ['a'], 'one engine freed for two opens that each need one slot')
  burst.retirer.dispose()
  background.retirer.dispose()
  foreground.retirer.dispose()
})
