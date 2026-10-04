import { useStore } from '@nanostores/react'
import { act, cleanup, render } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { toRuntimeMessage } from '@/lib/chat-runtime'
import { $notifications, clearNotifications } from '@/store/notifications'
import {
  $activeSessionId,
  $messages,
  $selectedStoredSessionId,
  setActiveSessionId,
  setAwaitingResponse,
  setBusy,
  setMessages,
  setSelectedStoredSessionId,
  setSessions
} from '@/store/session'
import { clearAllSessionStates } from '@/store/session-states'

import { useSessionStateCache } from '../use-session-state-cache'

import { usePromptActions } from '.'

// Real prompt hooks + real session-state cache + real stores; only the gateway edge is a stub.
const LINE = 'No messaging platform is configured for handoff'
const busyRef = { current: false }
let handle: { actions: ReturnType<typeof usePromptActions>; cache: ReturnType<typeof useSessionStateCache> }
let requestGateway: ReturnType<typeof vi.fn>
let createBackend: () => Promise<string | null>

function Harness() {
  const activeSessionId = useStore($activeSessionId)
  const selectedStoredSessionId = useStore($selectedStoredSessionId)

  const cache = useSessionStateCache({
    activeSessionId,
    selectedStoredSessionId,
    busyRef,
    setAwaitingResponse,
    setBusy,
    setMessages
  })

  const actions = usePromptActions({
    activeSessionId,
    ...cache,
    busyRef,
    branchCurrentSession: async () => false,
    createBackendSessionForSend: () => createBackend(),
    getRoutedStoredSessionId: () => $selectedStoredSessionId.get(),
    getRouteToken: () => `${$selectedStoredSessionId.get() ?? '/'}::`,
    handleSkinCommand: () => '',
    openMemoryGraph: () => undefined,
    refreshSessions: async () => undefined,
    requestGateway: requestGateway as never,
    resumeStoredSession: async () => {
      throw new Error('unexpected foreground resume')
    },
    startFreshSessionDraft: () => undefined,
    sttEnabled: false
  })

  handle = { actions, cache }

  return null
}

const transcriptText = () =>
  $messages
    .get()
    .map(message =>
      JSON.stringify(toRuntimeMessage(message).content)
    )
    .join('\n')

afterEach(() => {
  cleanup()
  clearAllSessionStates()
  clearNotifications()
  setActiveSessionId(null)
  setSelectedStoredSessionId(null)
  setSessions([])
  setMessages([])
  vi.clearAllMocks()
})

describe('/handoff with no configured platform, real stores', () => {
  const rejectHandoff = () => {
    requestGateway = vi.fn(async (method: string) => {
      if (method === 'handoff.request') {
        throw new Error("platform 'telegram' is not configured/enabled in the gateway")
      }

      return {} as never
    })
  }

  it('shows the notice in the transcript of an existing chat', async () => {
    rejectHandoff()
    setSelectedStoredSessionId('stored-B')
    setActiveSessionId('rt-B')
    render(<Harness />)
    act(() => {
      handle.cache.updateSessionState('rt-B', state => state, 'stored-B')
    })

    await act(async () => {
      await handle.actions.submitText('/handoff telegram')
    })

    expect(transcriptText()).toContain(LINE)
  })

  it('shows the notice on a fresh draft whose session is created by the command', async () => {
    rejectHandoff()

    createBackend = async () => {
      handle.cache.activeSessionIdRef.current = 'rt-new'
      handle.cache.selectedStoredSessionIdRef.current = 'stored-new'
      handle.cache.ensureSessionState('rt-new', 'stored-new')
      setActiveSessionId('rt-new')
      setSelectedStoredSessionId('stored-new')

      return 'rt-new'
    }

    render(<Harness />)

    await act(async () => {
      await handle.actions.submitText('/handoff telegram')
    })

    expect(transcriptText()).toContain(LINE)
  })

  // Live sequence: the idle runtime answers 4001 first, the chat is resumed onto a new runtime id, and only
  // then does the backend say the platform is not configured. The notice lands on the transcript on screen.
  it('survives a stale-runtime 4001: resumes, retries, and shows notice and toast on the visible chat', async () => {
    const calls: string[] = []
    let handoffTries = 0
    requestGateway = vi.fn(async (method: string, params?: Record<string, unknown>) => {
      calls.push(`${method}:${String(params?.session_id ?? '')}`)

      if (method === 'handoff.request') {
        handoffTries += 1

        throw new Error(
          handoffTries === 1 ? 'session not found' : "platform 'telegram' is not configured/enabled in the gateway"
        )
      }

      if (method === 'session.resume') {
        return { session_id: 'rt-B2' } as never
      }

      return {} as never
    })
    setSelectedStoredSessionId('stored-B')
    setActiveSessionId('rt-B')
    render(<Harness />)
    handle.cache.activeSessionIdRef.current = 'rt-B'
    handle.cache.selectedStoredSessionIdRef.current = 'stored-B'
    act(() => {
      handle.cache.updateSessionState('rt-B', state => state, 'stored-B')
    })

    await act(async () => {
      await handle.actions.submitText('/handoff telegram')
    })

    expect(handoffTries).toBe(2)
    expect(calls).toContain('handoff.request:rt-B2')
    expect($activeSessionId.get()).toBe('rt-B')
    expect(transcriptText()).toContain(LINE)
    expect($notifications.get().some(n => n.message.includes(LINE))).toBe(true)
  })
})
