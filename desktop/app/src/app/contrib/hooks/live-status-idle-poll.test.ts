import { act, cleanup, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { createClientSessionState } from '@/lib/chat-runtime'
import { resetLiveSync } from '@/store/live-sync'
import { clearAllSessionStates, publishSessionState } from '@/store/session-states'

import { liveStatusPollDue, useBackgroundSync } from './use-background-sync'

function mount(requestGateway: ReturnType<typeof vi.fn>) {
  return renderHook(() =>
    useBackgroundSync({
      activeConnectionId: 'local',
      activeGatewayProfile: 'default',
      activeIsMessaging: false,
      activeSessionId: null,
      activeStoredSessionId: null,
      freshDraftReady: false,
      gatewayState: 'open',
      refreshActiveTranscript: vi.fn(),
      refreshCronJobs: vi.fn(),
      refreshCurrentModel: vi.fn(),
      refreshFactrConfig: vi.fn(),
      refreshMessagingSessions: vi.fn(),
      refreshSessions: vi.fn(),
      requestGateway: requestGateway as never,
      updateSessionState: vi.fn() as never
    })
  )
}

const activeListCalls = (fn: ReturnType<typeof vi.fn>) =>
  fn.mock.calls.filter(c => c[0] === 'session.active_list').length

beforeEach(() => {
  vi.spyOn(document, 'hasFocus').mockReturnValue(true)
  vi.useFakeTimers()
})

afterEach(() => {
  cleanup()
  vi.useRealTimers()
  resetLiveSync()
  clearAllSessionStates()
})

describe('live status poll without change events (native engine)', () => {
  it('polls at most ~4 times a minute while idle, not every 1.5s', async () => {
    const request = vi.fn(async () => ({ sessions: [] }))

    mount(request)
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000)
    })

    // Was 41 (1 + one per 1.5s) before the idle gap.
    expect(activeListCalls(request)).toBeLessThanOrEqual(5)
    expect(activeListCalls(request)).toBeGreaterThanOrEqual(2)
  })

  it('keeps the 1.5s cadence while a session is working', async () => {
    const request = vi.fn(async () => ({ sessions: [] }))

    publishSessionState('rt', { ...createClientSessionState('stored'), busy: true })
    mount(request)
    await act(async () => {
      await vi.advanceTimersByTimeAsync(15_000)
    })

    expect(activeListCalls(request)).toBeGreaterThanOrEqual(9)
  })
})

describe('liveStatusPollDue', () => {
  it('is due when active, or when the idle gap has passed', () => {
    expect(liveStatusPollDue(1_000, 0, true)).toBe(true)
    expect(liveStatusPollDue(1_000, 0, false)).toBe(false)
    expect(liveStatusPollDue(20_000, 0, false)).toBe(true)
  })
})
