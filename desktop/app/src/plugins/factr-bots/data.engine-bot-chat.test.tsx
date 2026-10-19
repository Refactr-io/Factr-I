/**
 * A bot on its own engine: the primary's roster says `canonical_session: null` (it reads the primary's
 * state.db, which never sees that engine's chats). Host bots share one engine home, so the PRIMARY engine names
 * their Bot Chat from the shared registry (`bot.chat.get_many`, one batched call) and the roster fills the row in:
 * `/new` becomes `/compact`, the empty-chat Teach mount and the group start find the forever-chat. The roster
 * never talks to a bot's own route, because that would start the bot's engine (RAM).
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderHook, waitFor } from '@testing-library/react'
import type { ReactNode } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { isCanonicalChatOnScreen } from './canonical-chat'
import { $lastRoster, ROSTER_KEY, useRoster } from './data'
import { rememberEngineBotChat, resetEngineBotChats } from './engine-bot-chat'
import type { RosterRow } from './types'

const { hostMock, invalidateMock } = vi.hoisted(() => ({
  hostMock: {
    agents: vi.fn(),
    profileRoutes: undefined as unknown,
    request: vi.fn(),
    requestProfile: vi.fn(),
    state: { connectionId: { get: vi.fn(() => 'local') }, profile: { get: () => 'default' } }
  },
  invalidateMock: vi.fn()
}))

vi.mock('@factr/plugin-sdk', async () => {
  const { atom } = await import('nanostores')
  const { useQuery } = await import('@tanstack/react-query')

  return {
    BOT_CHAT_SESSION_HYDRATION_TIMEOUT_MS: 15_000,
    atom,
    host: hostMock,
    queryClient: { getQueryData: vi.fn(), invalidateQueries: invalidateMock },
    useQuery,
    useValue: (store: { get: () => unknown }) => store.get()
  }
})

vi.mock('./shared', () => ({ getPluginCtx: () => null, ID: 'factr-bots' }))

const ENGINE_CHAT = {
  id: 'engine-chat',
  message_count: 3,
  resolved_id: 'engine-chat',
  root_title: 'Bot Chat',
  title: 'Bot Chat'
}

const agent = (profile: string) => ({
  connectionId: 'local',
  connectionKind: 'local',
  connectionLabel: 'This device',
  handle: profile,
  profile
})

function mountRoster(profiles: Array<Partial<RosterRow>>, chats: unknown = { chats: {} }) {
  hostMock.request.mockImplementation(async (method: string) => (method === 'bot.chat.get_many' ? chats : { profiles }))
  hostMock.agents.mockResolvedValue({ agents: profiles.map(p => agent(String(p.name))) })

  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })

  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )

  return renderHook(() => useRoster(), { wrapper })
}

const rowOf = (rows: RosterRow[] | undefined, name: string) => rows?.find(row => row.name === name) as RosterRow
const manyCalls = () => hostMock.request.mock.calls.filter(call => call[0] === 'bot.chat.get_many')

beforeEach(() => {
  vi.clearAllMocks()
  resetEngineBotChats()
  $lastRoster.set([])
})

describe('the roster fills canonical_session from the primary engine', () => {
  it('asks once for every local bot, never touches a bot route, and fills the rows on the next read', async () => {
    const { result } = mountRoster(
      [
        { name: 'alpha', canonical_session: null },
        { name: 'beta', canonical_session: null }
      ],
      { chats: { alpha: ENGINE_CHAT, beta: null } }
    )

    await waitFor(() => expect(result.current.data).toBeTruthy())
    await waitFor(() => expect(invalidateMock).toHaveBeenCalledWith({ queryKey: ROSTER_KEY }))

    expect(manyCalls()).toHaveLength(1)
    expect(manyCalls()[0][1]).toEqual({ profiles: ['alpha', 'beta'] })

    const { data } = await result.current.refetch()
    const alpha = rowOf(data?.profiles as RosterRow[], 'alpha')

    expect(alpha.canonical_session).toMatchObject({ id: 'engine-chat', resolved_id: 'engine-chat' })
    expect(rowOf(data?.profiles as RosterRow[], 'beta').canonical_session ?? null).toBeNull()
    // ... which is exactly what the /new guard reads.
    expect(isCanonicalChatOnScreen(alpha, 'engine-chat')).toBe(true)
    expect(isCanonicalChatOnScreen(alpha, 'some-scratch-chat')).toBe(false)
    // RAM: a request on a bot's own route would start that bot's engine. The roster sent none.
    expect(hostMock.requestProfile).not.toHaveBeenCalled()
  })

  it("the primary's answer replaces its state.db pointer, and is asked once per TTL, not once per poll", async () => {
    const primary = { id: 'primary-chat', resolved_id: 'primary-chat', title: 'Bot Chat' }
    const { result } = mountRoster([{ name: 'beta', canonical_session: primary }], { chats: { beta: ENGINE_CHAT } })

    await waitFor(() => expect(invalidateMock).toHaveBeenCalled())
    await result.current.refetch()

    const { data } = await result.current.refetch()

    expect(manyCalls()).toHaveLength(1)
    // The chat the engine opens is the one the /new guard must recognise, not the primary's older pointer.
    expect(rowOf(data?.profiles as RosterRow[], 'beta').canonical_session).toMatchObject({ id: 'engine-chat' })
  })

  it("keeps the primary's pointer when the registry does not know the bot", async () => {
    const primary = { id: 'primary-chat', resolved_id: 'primary-chat', title: 'Bot Chat' }
    const { result } = mountRoster([{ name: 'beta', canonical_session: primary }], { chats: { beta: null } })

    await waitFor(() => expect(manyCalls()).toHaveLength(1))

    const { data } = await result.current.refetch()

    expect(rowOf(data?.profiles as RosterRow[], 'beta').canonical_session).toEqual(primary)
  })

  it('stays as it was, without noise, when the primary cannot answer (older engine)', async () => {
    const { result } = mountRoster([{ name: 'alpha', canonical_session: null }])

    hostMock.request.mockImplementation(async (method: string) => {
      if (method === 'bot.chat.get_many') {
        throw new Error('method not found')
      }

      return { profiles: [{ name: 'alpha', canonical_session: null }] }
    })

    await waitFor(() => expect(result.current.data).toBeTruthy())
    await result.current.refetch()

    expect(rowOf(result.current.data?.profiles as RosterRow[], 'alpha').canonical_session ?? null).toBeNull()
    expect(invalidateMock).not.toHaveBeenCalled()
    expect(hostMock.requestProfile).not.toHaveBeenCalled()
  })

  it('boot creates nothing: the roster only reads (no ensure, claim, session.create or prompt for default or any bot)', async () => {
    const { result } = mountRoster(
      [
        { name: 'default', canonical_session: null },
        { name: 'alpha', canonical_session: null }
      ],
      { chats: { default: null, alpha: null } }
    )

    await waitFor(() => expect(manyCalls()).toHaveLength(1))
    await result.current.refetch()

    const methods = new Set([...hostMock.request.mock.calls, ...hostMock.requestProfile.mock.calls].map(call => call[0] === 'object' ? call[1] : call[0]))

    for (const writer of ['bot.chat.ensure', 'bot.chat.claim', 'session.create', 'session.title', 'prompt.submit']) {
      expect(methods.has(writer)).toBe(false)
    }

    expect(hostMock.requestProfile).not.toHaveBeenCalled()
  })

  it('asks about a bot with no route of its own by profile name, through the primary, so no engine is chosen by accident', async () => {
    hostMock.request.mockImplementation(async (method: string) =>
      method === 'bot.chat.get_many'
        ? { chats: { alpha: ENGINE_CHAT } }
        : { profiles: [{ name: 'alpha', canonical_session: null }] }
    )
    hostMock.agents.mockRejectedValue(new Error('no union roster on this build'))

    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })

    const { result } = renderHook(() => useRoster(), {
      wrapper: ({ children }: { children: ReactNode }) => (
        <QueryClientProvider client={client}>{children}</QueryClientProvider>
      )
    })

    await waitFor(() => expect(manyCalls()).toHaveLength(1))
    expect(manyCalls()[0][1]).toEqual({ profiles: ['alpha'] })

    const { data } = await result.current.refetch()

    expect(rowOf(data?.profiles as RosterRow[], 'alpha').canonical_session).toMatchObject({ id: 'engine-chat' })
    expect(hostMock.requestProfile).not.toHaveBeenCalled()
  })

  it('a chat learned when the bot was opened (a container keeps its own engine home) fills its row, still without spawning', async () => {
    const { result } = mountRoster([{ name: 'boxed', canonical_session: null }], { chats: { boxed: null } })

    rememberEngineBotChat({ name: 'boxed' }, ENGINE_CHAT)

    await waitFor(() => expect(result.current.data).toBeTruthy())

    const { data } = await result.current.refetch()

    // The primary does not know the container's chat (null) and does not erase what opening it taught.
    expect(rowOf(data?.profiles as RosterRow[], 'boxed').canonical_session).toMatchObject({ id: 'engine-chat' })
    expect(hostMock.requestProfile).not.toHaveBeenCalled()
  })
})
