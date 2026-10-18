/**
 * A bot on its own engine: the primary's roster says `canonical_session: null` (it reads the primary's
 * state.db, which never sees that engine's chats), so the roster asks the bot's OWN engine for its Bot Chat
 * (`bot.chat.get`) and fills the row in. That is what lets `/new` become `/compact`, the empty-chat Teach
 * mount and the group start find the forever-chat of an engine-backed bot.
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderHook, waitFor } from '@testing-library/react'
import type { ReactNode } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { isCanonicalChatOnScreen } from './canonical-chat'
import { $lastRoster, resetEngineBotChats, ROSTER_KEY, useRoster } from './data'
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

function mountRoster(profiles: Array<Partial<RosterRow>>) {
  hostMock.request.mockResolvedValue({ profiles })
  hostMock.agents.mockResolvedValue({ agents: profiles.map(p => agent(String(p.name))) })

  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })

  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )

  return renderHook(() => useRoster(), { wrapper })
}

const rowOf = (rows: RosterRow[] | undefined, name: string) => rows?.find(row => row.name === name) as RosterRow

beforeEach(() => {
  vi.clearAllMocks()
  resetEngineBotChats()
  $lastRoster.set([])
})

describe('a bot whose roster row has no canonical_session', () => {
  it('asks its own engine for the Bot Chat and fills the row on the next roster read', async () => {
    hostMock.requestProfile.mockResolvedValue({ canonical_session: ENGINE_CHAT, created: false })

    const { result } = mountRoster([{ name: 'alpha', canonical_session: null }])

    await waitFor(() => expect(result.current.data).toBeTruthy())
    await waitFor(() => expect(invalidateMock).toHaveBeenCalledWith({ queryKey: ROSTER_KEY }))

    // The question went to alpha's own route (a profile route, never the primary's ambient request).
    const [route, method] = hostMock.requestProfile.mock.calls[0]

    expect(method).toBe('bot.chat.get')
    expect(route).toMatchObject({ connectionId: 'local' })

    const { data } = await result.current.refetch()
    const alpha = rowOf(data?.profiles as RosterRow[], 'alpha')

    expect(alpha.canonical_session).toMatchObject({ id: 'engine-chat', resolved_id: 'engine-chat' })
    // ... which is exactly what the /new guard reads.
    expect(isCanonicalChatOnScreen(alpha, 'engine-chat')).toBe(true)
    expect(isCanonicalChatOnScreen(alpha, 'some-scratch-chat')).toBe(false)
  })

  it("the engine's registry replaces the primary's pointer, and asks once per TTL, not once per poll", async () => {
    hostMock.requestProfile.mockResolvedValue({ canonical_session: ENGINE_CHAT })

    const primary = { id: 'primary-chat', resolved_id: 'primary-chat', title: 'Bot Chat' }
    const { result } = mountRoster([{ name: 'beta', canonical_session: primary }])

    await waitFor(() => expect(invalidateMock).toHaveBeenCalled())
    await result.current.refetch()
    const { data } = await result.current.refetch()

    expect(hostMock.requestProfile.mock.calls.filter(call => call[1] === 'bot.chat.get')).toHaveLength(1)
    // The chat the engine opens is the one the /new guard must recognise, not the primary's older pointer.
    expect(rowOf(data?.profiles as RosterRow[], 'beta').canonical_session).toMatchObject({ id: 'engine-chat' })
  })

  it('keeps the primary\'s pointer when the engine says it has no Bot Chat', async () => {
    hostMock.requestProfile.mockResolvedValue({ canonical_session: null })

    const primary = { id: 'primary-chat', resolved_id: 'primary-chat', title: 'Bot Chat' }
    const { result } = mountRoster([{ name: 'beta', canonical_session: primary }])

    await waitFor(() => expect(hostMock.requestProfile).toHaveBeenCalled())
    const { data } = await result.current.refetch()

    expect(rowOf(data?.profiles as RosterRow[], 'beta').canonical_session).toEqual(primary)
  })

  it('never asks for a bot that has no route of its own: that request would reach the active gateway', async () => {
    hostMock.request.mockResolvedValue({ profiles: [{ name: 'alpha', canonical_session: null }] })
    hostMock.agents.mockRejectedValue(new Error('no union roster on this build'))

    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })

    const { result } = renderHook(() => useRoster(), {
      wrapper: ({ children }: { children: ReactNode }) => (
        <QueryClientProvider client={client}>{children}</QueryClientProvider>
      )
    })

    await waitFor(() => expect(result.current.data).toBeTruthy())
    await result.current.refetch()

    expect(hostMock.request.mock.calls.map(call => call[0])).not.toContain('bot.chat.get')
    expect(hostMock.requestProfile).not.toHaveBeenCalled()
  })

  it('stays null, without noise, when the engine cannot answer (older engine, Python gateway)', async () => {
    hostMock.requestProfile.mockRejectedValue(new Error('method not found'))

    const { result } = mountRoster([{ name: 'alpha', canonical_session: null }])

    await waitFor(() => expect(result.current.data).toBeTruthy())
    await waitFor(() => expect(hostMock.requestProfile).toHaveBeenCalled())
    await result.current.refetch()

    expect(rowOf(result.current.data?.profiles as RosterRow[], 'alpha').canonical_session ?? null).toBeNull()
    expect(invalidateMock).not.toHaveBeenCalled()
  })

  it('does not ask for a bot the engine says has no Bot Chat yet to be "learned" (null stays null)', async () => {
    hostMock.requestProfile.mockResolvedValue({ canonical_session: null, created: false })

    const { result } = mountRoster([{ name: 'alpha', canonical_session: null }])

    await waitFor(() => expect(hostMock.requestProfile).toHaveBeenCalled())
    await result.current.refetch()

    expect(rowOf(result.current.data?.profiles as RosterRow[], 'alpha').canonical_session ?? null).toBeNull()
  })
})
