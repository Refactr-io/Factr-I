/**
 * Open Bot Chat for a bot on its own engine. That engine's `session.list` has no title lookup and lists
 * only the newest rows, so the "Bot Chat" row can sit past the window while the roster's pointer says it
 * exists: the old code failed closed ("Could not confirm <bot>'s Bot Chat registry"). The engine's own
 * registry (`bot.chat.get`) is the authority: its row is opened, and "none" lets the chat be created.
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

const calls: string[] = []

function respondWith(handler: (method: string) => unknown) {
  calls.length = 0
  requestForBotMock.mockImplementation(async (_bot: unknown, method: string) => {
    calls.push(method)

    return handler(method)
  })
}

async function loadModule() {
  vi.resetModules()

  return import('./canonical-chat')
}

const rosterBot = { name: 'ops', canonical_session: { id: 'forever', resolved_id: 'forever' } } as RosterRow

beforeEach(() => {
  vi.clearAllMocks()
  hostMock.openSession.mockResolvedValue(undefined)
})

describe('a bot on its own engine', () => {
  it("opens the engine registry's row and never trusts a title scan of the shared store", async () => {
    respondWith(method => {
      if (method === 'bot.chat.get') {
        return { canonical_session: { id: 'forever', message_count: 40, resolved_id: 'forever', title: 'Bot Chat' } }
      }

      throw new Error(`unexpected ${method}`)
    })

    const { openBotCanonicalChat } = await loadModule()
    const opened = await openBotCanonicalChat(rosterBot)

    expect(opened).toEqual({ openedId: 'forever', registryId: 'forever' })
    expect(calls).toEqual(['bot.chat.get'])
    expect(hostMock.openSession.mock.calls[0][0]).toBe('forever')
  })

  it("remembers the engine's answer for the roster, so a bot with its own engine home shows its chat without a spawn", async () => {
    respondWith(() => ({ canonical_session: { id: 'forever', resolved_id: 'forever', title: 'Bot Chat' } }))

    const { openBotCanonicalChat } = await loadModule()
    const { cachedEngineBotChat } = await import('./engine-bot-chat')

    await openBotCanonicalChat(rosterBot)

    expect(cachedEngineBotChat(rosterBot)?.chat?.id).toBe('forever')
  })

  it('an engine that says "no Bot Chat" settles absence: the chat is created and registered, not refused', async () => {
    respondWith(method => {
      if (method === 'bot.chat.get') {
        return { canonical_session: null }
      }

      if (method === 'session.create') {
        return { session_id: 'runtime-1', stored_session_id: 'minted' }
      }

      if (method === 'bot.chat.claim') {
        return { canonical_session: { id: 'minted' }, claimed: true }
      }

      return {}
    })

    const { openBotCanonicalChat } = await loadModule()

    await expect(openBotCanonicalChat(rosterBot)).resolves.toBeTruthy()
    expect(calls).toEqual(['bot.chat.get', 'bot.chat.get', 'session.create', 'session.title', 'bot.chat.claim'])
  })

  it('a chat that loses the claim race is abandoned and the registered one is adopted', async () => {
    respondWith(method => {
      if (method === 'bot.chat.get') {
        return { canonical_session: null }
      }

      if (method === 'session.create') {
        return { session_id: 'mine', stored_session_id: 'mine' }
      }

      if (method === 'bot.chat.claim') {
        return { canonical_session: { id: 'winner', resolved_id: 'winner' }, claimed: false }
      }

      return {}
    })

    const { createCanonicalChat } = await loadModule()

    await expect(createCanonicalChat(rosterBot)).resolves.toBe('winner')
    expect(hostMock.openSession.mock.calls.map(call => call[0])).toEqual(['winner'])
    expect(calls).not.toContain('prompt.submit')
  })

  it('an engine that cannot answer falls back to the title lookup, and keeps the fail-closed guard', async () => {
    respondWith(method => {
      if (method === 'session.list') {
        return { sessions: [] }
      }

      throw new Error('method not found')
    })

    const { openBotCanonicalChat } = await loadModule()

    await expect(openBotCanonicalChat(rosterBot)).rejects.toThrow(/Could not confirm ops's Bot Chat registry/)
    expect(calls).toEqual(['bot.chat.get', 'session.list'])
    expect(calls).not.toContain('session.create')
  })
})
