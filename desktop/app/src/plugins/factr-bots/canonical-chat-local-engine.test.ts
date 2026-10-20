import { beforeEach, expect, it, vi } from 'vitest'

const { hostMock, events } = vi.hoisted(() => ({
  events: [] as string[],
  hostMock: { request: vi.fn(), requestProfile: vi.fn(), retainProfile: vi.fn(), openSession: vi.fn() }
}))
vi.mock('./relay', () => ({ refreshBotRelayRosters: vi.fn(async () => { events.push('roster.sync') }) }))

vi.mock('@factr/plugin-sdk', () => ({ host: hostMock, BOT_CHAT_SESSION_HYDRATION_TIMEOUT_MS: 60_000 }))
vi.mock('./data', () => ({
  $botMeta: { get: () => ({}), set: vi.fn() },
  botMetaKey: () => 'alpha',
  persistBotMetaSnapshot: vi.fn(),
  botOwner: (owner: string | { name: string; route?: unknown }) =>
    typeof owner === 'string'
      ? { bot: { name: owner }, name: owner, key: owner, route: null }
      : { bot: owner, name: owner.name, key: owner.name, route: owner.route || null }
}))
vi.mock('./shared', () => ({ getPluginCtx: () => null }))

beforeEach(() => {
  vi.resetModules()
  vi.clearAllMocks()
  events.length = 0
})

it('births the intro in the private engine and retains its connection through title, claim and submit', async () => {
  let retained = false
  hostMock.retainProfile.mockImplementation(async (profile: string) => {
    expect(profile).toBe('alpha')
    retained = true
    return () => {
      retained = false
      events.push('release')
    }
  })
  hostMock.request.mockImplementation(() => {
    throw new Error('the primary engine must not create this chat')
  })
  hostMock.requestProfile.mockImplementation(async (profile: string, method: string) => {
    expect(profile).toBe('alpha')
    expect(retained).toBe(true)
    events.push(method)
    if (method === 'bot.chat.get') { return { canonical_session: null } }
    if (method === 'session.create') { return { session_id: 'alpha-chat', stored_session_id: 'alpha-chat' } }
    if (method === 'bot.chat.claim') { return { canonical_session: { id: 'alpha-chat' } } }
    return {}
  })
  hostMock.openSession.mockResolvedValue(undefined)
  const { createCanonicalChat } = await import('./canonical-chat')
  expect(await createCanonicalChat('alpha', { kickoff: true })).toBe('alpha-chat')
  expect(events).toEqual([
    'bot.chat.get',
    'session.create',
    'session.title',
    'bot.chat.claim',
    'roster.sync',
    'prompt.submit',
    'release'
  ])
  expect(hostMock.openSession).toHaveBeenCalledWith(
    'alpha-chat',
    expect.objectContaining({ profile: 'alpha', workspaceMode: 'bots' })
  )
  expect(hostMock.openSession.mock.calls[0][1]).not.toHaveProperty('route')
  hostMock.requestProfile.mockImplementation(async (profile: string, method: string) => {
    expect(profile).toBe('alpha')
    expect(method).toBe('bot.chat.get')
    return { canonical_session: { id: 'alpha-chat', message_count: 2, title: 'Bot Chat' } }
  })
  const { openBotCanonicalChat } = await import('./canonical-chat')
  expect(await openBotCanonicalChat('alpha')).toEqual({ openedId: 'alpha-chat', registryId: 'alpha-chat' })
  expect(hostMock.openSession).toHaveBeenLastCalledWith(
    'alpha-chat',
    expect.objectContaining({ profile: 'alpha', awaitHydration: true, expectHistory: true })
  )
  expect(hostMock.request).not.toHaveBeenCalled()
})
