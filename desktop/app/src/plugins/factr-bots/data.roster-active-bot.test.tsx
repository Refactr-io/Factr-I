/**
 * The roster after "Bot created". Creating a bot makes it the active profile, so
 * the active gateway becomes that bot's own backend, and a backend whose Python
 * half is not running answers `profiles.list` with only its default row. The
 * roster must read the list from the root profile's backend instead.
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderHook, waitFor } from '@testing-library/react'
import type { ReactNode } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { $lastRoster, useRoster } from './data'

const { hostMock, profileName } = vi.hoisted(() => ({
  profileName: { value: 'default' },
  hostMock: {
    agents: vi.fn(),
    profileRoutes: undefined as unknown,
    request: vi.fn(),
    requestProfile: vi.fn(),
    state: { connectionId: { get: vi.fn(() => 'local') }, profile: { get: (): string => 'default' } }
  }
}))

vi.mock('@factr/plugin-sdk', async () => {
  const { atom } = await import('nanostores')
  const { useQuery } = await import('@tanstack/react-query')

  return {
    atom,
    host: hostMock,
    queryClient: { getQueryData: vi.fn(), invalidateQueries: vi.fn() },
    useQuery,
    useValue: (store: { get: () => unknown }) => store.get()
  }
})

vi.mock('./shared', () => ({ getPluginCtx: () => null, ID: 'factr-bots' }))

const STUB = { bot_mode_protocol: true, profiles: [{ name: 'default' }] }
const REAL = { bot_mode_protocol: true, profiles: [{ name: 'default' }, { name: 'scribe' }] }

async function roster() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })

  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )

  const { result } = renderHook(() => useRoster(), { wrapper })

  await waitFor(() => expect(result.current.data).toBeTruthy())

  return (result.current.data?.profiles ?? []).map(row => row.name)
}

beforeEach(() => {
  vi.clearAllMocks()
  $lastRoster.set([])
  profileName.value = 'default'
  hostMock.state.profile.get = () => profileName.value
  hostMock.state.connectionId.get.mockReturnValue('local')
  hostMock.agents.mockRejectedValue(new Error('no union roster'))
})

describe('roster while a bot is the active profile', () => {
  it('asks the root profile backend, not the new bot backend that only stubs', async () => {
    profileName.value = 'scribe'
    hostMock.request.mockResolvedValue(STUB)
    hostMock.requestProfile.mockResolvedValue(REAL)

    expect(await roster()).toEqual(['default', 'scribe'])
    expect(hostMock.requestProfile).toHaveBeenCalledWith(
      { connectionId: 'local', mode: 'local', profile: 'default', targetProfile: 'default' },
      'profiles.list',
      {}
    )
    expect(hostMock.request).not.toHaveBeenCalled()
  })

  it('falls back to the active gateway when the root backend cannot be reached', async () => {
    profileName.value = 'scribe'
    hostMock.request.mockResolvedValue(REAL)
    hostMock.requestProfile.mockRejectedValue(new Error('root backend not up'))

    expect(await roster()).toEqual(['default', 'scribe'])
    expect(hostMock.request).toHaveBeenCalledWith('profiles.list', {})
  })

  it('keeps the active gateway door when the default profile is active', async () => {
    hostMock.request.mockResolvedValue(REAL)

    expect(await roster()).toEqual(['default', 'scribe'])
    expect(hostMock.requestProfile).not.toHaveBeenCalled()
  })

  it('never routes a remote connection through the local root profile', async () => {
    profileName.value = 'scribe'
    hostMock.state.connectionId.get.mockReturnValue('spark')
    hostMock.request.mockResolvedValue(REAL)

    expect(await roster()).toEqual(['default', 'scribe'])
    expect(hostMock.requestProfile).not.toHaveBeenCalled()
  })
})
