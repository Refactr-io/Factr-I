import assert from 'node:assert/strict'

import { test } from 'vitest'

import { createPoolRetirer } from './pool-retire'
import { createPoolRetirementClient } from './pool-retire-http'
import { LocalBackendSpawnCoordinator } from './pool-spawn-coordinator'

const entry = (kind = 'factr') => ({ kind, port: 4242, process: {}, token: 't' })

test('a missing retirement door never grants authority from a session snapshot', async () => {
  const client = createPoolRetirementClient(async () => {
    throw new Error('404')
  })
  assert.equal(await client.prepare('a', entry()), null)
  assert.equal(await client.commit('a', entry(), 'invented'), false)
})

test('both runtimes use their own permit, with a busy answer final and cancellation explicit', async () => {
  for (const kind of ['factr', 'python']) {
    const calls: string[] = []
    const client = createPoolRetirementClient(async (url, _token, options) => {
      assert.ok(url.endsWith('/api/health/retirement'))
      calls.push(options.body.action)
      return { ok: true, idle: true, token: 'owned-permit' }
    })
    const token = await client.prepare('a', entry(kind))
    assert.equal(token, 'owned-permit')
    assert.equal(await client.commit('a', entry(kind), token!), true)
    await client.cancel('a', entry(kind), token!)
    assert.deepEqual(calls, ['prepare', 'commit', 'cancel'])
    const busy = createPoolRetirementClient(async () => ({ ok: true, idle: false }))
    assert.equal(await busy.prepare('a', entry(kind)), null)
  }
})

test('commit waits for the drain acknowledgment', async () => {
  const calls: string[] = []
  const client = createPoolRetirementClient(async (_url, _token, options) => {
    calls.push(options.body.action)
    return { ok: true, idle: options.body.action === 'status', token: 'permit' }
  })
  assert.equal(await client.commit('a', entry(), 'permit'), true)
  assert.deepEqual(calls, ['commit', 'status'])
})

test('stale renderer busy flags cannot strand an idle Rust engine, and slots wait for real exit', async () => {
  const coordinator = new LocalBackendSpawnCoordinator(3)
  const pool = new Map(['a', 'b', 'c'].map((key, i) => [key, { ...entry(), lastActiveAt: i, activeTurn: true }]))
  const release = new Map<string, () => void>()
  for (const key of pool.keys()) { release.set(key, await coordinator.acquire(key)) }
  let exited!: () => void
  const exit = new Promise<void>(resolve => {
    exited = resolve
  })
  const stopped: string[] = []
  const permits: string[] = []
  const retirer = createPoolRetirer({
    coordinator,
    pool,
    prepare: async key => {
      permits.push(key)
      return key === 'a' ? null : 'permit'
    },
    commit: async () => true,
    cancel: async () => {},
    stopBackend: async key => {
      stopped.push(key)
      await exit
      pool.delete(key)
      release.get(key)!()
    }
  })
  const d = coordinator.request('d', { timeoutMs: 3000 })
  const e = coordinator.request('e', { timeoutMs: 3000 })
  let granted = 0
  const dDone = d.acquired.then(r => {
    granted++
    return r
  })
  await new Promise(resolve => setTimeout(resolve, 20))
  assert.deepEqual(stopped, ['b'])
  assert.equal(granted, 0, 'drain acknowledgment is not process exit')
  let reopened = false
  const reopen = retirer.waitForRetirement('b').then(() => {
    reopened = true
  })
  await Promise.resolve()
  assert.equal(reopened, false)
  exited()
  const rd = await dDone
  const re = await e.acquired
  await reopen
  assert.equal(coordinator.activeCount, 3)
  assert.deepEqual(stopped, ['b', 'c'])
  assert.deepEqual(permits, ['a', 'b', 'a', 'c'])
  rd()
  re()
  release.get('a')!()
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
      ['a', 'b', 'c'].map((key, i) => [
        key,
        { kind: 'factr', lastActiveAt: Date.now() - 1000 * (3 - i), port: 4000 + i, process: {}, token: 't' }
      ])
    )

    const stopped: string[] = []

    const retirer = createPoolRetirer({
      coordinator: new LocalBackendSpawnCoordinator(3),
      pool,
      stopBackend: async key => {
        stopped.push(key)
        pool.delete(key)
      },
      ...createPoolRetirementClient(async () => ({ ok: true, idle: true, token: 'permit' }))
    })

    return { pool, retirer, stopped }
  }

  // All three were touched seconds ago (inside the keepalive window) and none runs a turn.
  const background = build()

  assert.deepEqual(await background.retirer.evictTo(2, evictionFreshMs('background', 240_000)), [])
  assert.deepEqual(background.stopped, [])

  const foreground = build()

  assert.deepEqual(
    await foreground.retirer.evictTo(2, evictionFreshMs('foreground', 240_000)),
    ['a'],
    'the least recently used goes'
  )
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
