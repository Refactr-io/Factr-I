/**
 * The packaged topology: a primary engine plus one engine per bot, all on ONE engine home (one session store, one
 * pointer table). The window's calls are routed by how the bot row is routed: an unrouted row reaches whichever
 * gateway is ACTIVE, which is not necessarily the bot's own engine (it was the last bot's, or the primary). The
 * registry must therefore be keyed by the profile the call NAMES, not by the engine that serves it, or bots end up
 * with each other's chats (run 2: bot_chat:default = alpha's chat, bot_chat:beta = gamma's, opens cross-wired).
 *
 * The fake engines implement the engine's `bot.chat.*` contract (pointer per named profile, shared store; only the
 * profile's own engine mints) and the test routes every call the way the window would.
 */

import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { RosterRow } from './types'

const { hostMock, requestForBotMock } = vi.hoisted(() => ({
  hostMock: { openSession: vi.fn(), request: vi.fn() },
  requestForBotMock: vi.fn()
}))

vi.mock('@factr/plugin-sdk', () => ({ BOT_CHAT_SESSION_HYDRATION_TIMEOUT_MS: 15_000, host: hostMock }))

vi.mock('./routing', () => ({
  backendTargetProfile: (route: { targetProfile?: string } | null, name: string) => route?.targetProfile ?? name,
  botConnectionRoute: () => null,
  botRosterMeta: () => ({}),
  botWorkspaceOwnerKey: (bot: { name?: string } | null) => `bot:${bot?.name || 'default'}`,
  requestForBot: requestForBotMock
}))

vi.mock('./data', () => ({
  $botMeta: { get: () => ({}), set: vi.fn() },
  botMetaKey: (bot: { name?: string }) => bot?.name ?? '',
  botOwner: (owner: RosterRow | string) =>
    typeof owner === 'string'
      ? { bot: { name: owner }, key: owner, name: owner, route: null }
      : { bot: owner, key: owner?.name, name: owner?.name, route: null },
  persistBotMetaSnapshot: vi.fn(),
  saveBotMeta: vi.fn()
}))

vi.mock('./shared', () => ({ getPluginCtx: () => null }))

/** One shared store and pointer table, three engines in front of it. */
function topology() {
  const pointers = new Map<string, string>()
  const sessions = new Map<string, { title: string; engine: string }>()
  const served: Array<{ engine: string; method: string; params: Record<string, unknown> }> = []
  let active = 'primary'
  let n = 0

  const row = (id: string) => ({ id, message_count: 0, resolved_id: id, root_title: 'Bot Chat', title: 'Bot Chat' })

  requestForBotMock.mockImplementation(async (_bot: unknown, method: string, params: Record<string, unknown> = {}) => {
    const engine = active // every row here is unrouted: the request reaches the ACTIVE gateway

    served.push({ engine, method, params })

    if (method === 'bot.chat.get') {
      const id = pointers.get(String(params.profile ?? engine))

      return { canonical_session: id && sessions.has(id) ? row(id) : null }
    }

    if (method === 'bot.chat.claim') {
      const key = String(params.profile ?? engine)
      const existing = pointers.get(key)

      if (existing && sessions.has(existing)) {
        return { canonical_session: row(existing), claimed: false }
      }

      pointers.set(key, String(params.session_id))

      return { canonical_session: row(String(params.session_id)), claimed: true }
    }

    if (method === 'session.create') {
      const id = `chat-${++n}-on-${engine}`

      sessions.set(id, { engine, title: 'Bot Chat' })

      return { session_id: id, stored_session_id: id }
    }

    return {}
  })

  return { pointers, sessions, served, setActive: (engine: string) => (active = engine) }
}

async function loadModule() {
  vi.resetModules()

  return import('./canonical-chat')
}

beforeEach(() => {
  vi.clearAllMocks()
  hostMock.openSession.mockResolvedValue(undefined)
})

describe('a primary plus per-bot engines on one store', () => {
  it('registers each bot under its own name whichever engine the call reached, and opens each bot its own chat', async () => {
    const world = topology()
    const { createCanonicalChat, openBotCanonicalChat } = await loadModule()

    // Creation happens while ANOTHER engine is the active gateway (the last bot, then the primary).
    world.setActive('gamma')
    const alpha = await createCanonicalChat('alpha')

    world.setActive('primary')
    const beta = await createCanonicalChat('beta')

    world.setActive('alpha')
    const gamma = await createCanonicalChat('gamma')

    expect(Object.fromEntries(world.pointers)).toEqual({ alpha, beta, gamma })
    expect(new Set([alpha, beta, gamma]).size).toBe(3)
    // Nothing was filed under the engine that happened to serve the call.
    expect(world.pointers.has('default') || world.pointers.has('primary') || world.pointers.has('gamma-engine')).toBe(false)

    // Every registry call named its profile.
    for (const call of world.served.filter(entry => entry.method.startsWith('bot.chat.'))) {
      expect(typeof call.params.profile).toBe('string')
    }

    // Opening each bot (from yet another active engine) opens ITS chat, never a neighbour's.
    hostMock.openSession.mockClear()
    world.setActive('beta')

    for (const [name, chat] of [
      ['alpha', alpha],
      ['beta', beta],
      ['gamma', gamma]
    ] as const) {
      await expect(openBotCanonicalChat(name)).resolves.toEqual({ openedId: chat, registryId: chat })
    }

    expect(hostMock.openSession.mock.calls.map(call => call[0])).toEqual([alpha, beta, gamma])
  })

  it('an open does not wait on history the empty chat does not have (no endless "Waking up")', async () => {
    const world = topology()
    const { createCanonicalChat, openBotCanonicalChat } = await loadModule()

    await createCanonicalChat('alpha')
    hostMock.openSession.mockClear()
    await openBotCanonicalChat('alpha')

    expect(hostMock.openSession.mock.calls[0][1]).toMatchObject({ awaitHydration: true, expectHistory: false })
    expect(world.sessions.size).toBe(1)
  })
})
