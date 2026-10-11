// The gateway `error` event (tui_gateway/prompt_turn.py) carries only a
// message — no error_surface. The dispatcher must still classify the two
// refusals it can mean so the card and toast read like a classified turn:
// plain words up front, raw text demoted to the detail line, and Retry gone
// where retrying reproduces the failure.
import { afterEach, describe, expect, it, vi } from 'vitest'

import { $notifications } from '@/store/notifications'

import { handleStatusEvent } from './status'
import type { GatewayEventContext } from './types'

vi.mock('@/store/native-notifications', () => ({ dispatchNativeNotification: vi.fn() }))
vi.mock('@/store/onboarding', () => ({ requestDesktopOnboarding: vi.fn() }))

const OWNED_REFUSAL =
  'Session 20260909_095312_6b93f5 already has a live owner (tui, pid 32977, lease age 22m). ' +
  'Attach through a compatible owner, or close the session in its owning surface before resuming here.'

function errorContext(message: string, extra: Record<string, unknown> = {}) {
  const failAssistantMessage = vi.fn()
  const payload = { message, ...extra } as GatewayEventContext['payload']

  const ctx: GatewayEventContext = {
    deps: {
      compactedTurnRef: { current: new Set<string>() },
      failAssistantMessage,
      flushQueuedDeltas: vi.fn(),
      hydrateFromStoredSession: vi.fn(),
      queryClient: { invalidateQueries: vi.fn() },
      sessionStateByRuntimeIdRef: { current: new Map() },
      updateSessionState: vi.fn()
    } as unknown as GatewayEventContext['deps'],
    event: { payload, session_id: 'sess-1', type: 'error' },
    explicitSid: 'sess-1',
    fromActiveSource: () => true,
    isActiveEvent: false,
    occurredAt: 1_700_000_100,
    payload,
    scheduleConfigRefresh: vi.fn(),
    sessionId: 'sess-1'
  }

  return { ctx, failAssistantMessage }
}

afterEach(() => {
  $notifications.set([])
})

describe('gateway `error` event → error card + toast', () => {
  it('honours a descriptor on the event: the engine-restart card keeps Retry', () => {
    const detail = 'The Factr-I engine restarted while this reply was running, so the reply was lost.'

    const { ctx, failAssistantMessage } = errorContext(detail, {
      error_surface: { code: 'engine_restarted', layer: 'gateway', retryable: true }
    })

    expect(handleStatusEvent(ctx)).toBe(true)
    expect(failAssistantMessage).toHaveBeenCalledWith(
      'sess-1',
      detail,
      1_700_000_100,
      expect.objectContaining({ code: 'engine_restarted', retryable: true })
    )

    const toast = $notifications.get()[0]

    expect(toast.message).toMatch(/connection to the engine was interrupted/i)
    expect(toast.message).toMatch(/not completed/i)
    expect(toast.detail).toBe(detail)
  })

  it('stamps SESSION_NOT_OWNED on a live-owner refusal so the card drops Retry', () => {
    const { ctx, failAssistantMessage } = errorContext(OWNED_REFUSAL)

    expect(handleStatusEvent(ctx)).toBe(true)

    expect(failAssistantMessage).toHaveBeenCalledWith(
      'sess-1',
      OWNED_REFUSAL,
      1_700_000_100,
      expect.objectContaining({ code: 'SESSION_NOT_OWNED', retryable: false })
    )
  })

  it('toasts the plain explanation, not the lease jargon, and keeps the raw text as detail', () => {
    const { ctx } = errorContext(OWNED_REFUSAL)

    handleStatusEvent(ctx)

    const toast = $notifications.get()[0]

    expect(toast.message).toBeTruthy()
    expect(toast.message).not.toMatch(/lease|pid|live owner/i)
    expect(toast.detail).toBe(OWNED_REFUSAL)
    expect(toast.action).toBeUndefined()
  })

  it("keeps the server's own plain copy as the toast message when no code was recovered", () => {
    // tui_gateway/user_messages.py already writes actionable sentences for
    // pre-turn failures; the generic "couldn't finish" gloss must not bury them.
    const serverCopy = 'Factr could not start the assistant for this chat. Check your model settings and try again.'
    const { ctx, failAssistantMessage } = errorContext(serverCopy)

    handleStatusEvent(ctx)

    expect(failAssistantMessage).toHaveBeenCalledWith('sess-1', serverCopy, 1_700_000_100, null)
    const toast = $notifications.get()[0]
    expect(toast.message).toBe(serverCopy)
    expect(toast.detail).toBeUndefined()
  })

  it('reads a raw provider 401 dump as a plain sentence and keeps the tidied dump as detail', () => {
    const raw = [
      'OpenAI-compatible chat request failed',
      '  endpoint: https://api.example.com/v1/chat/completions',
      '  model: gpt-x',
      '  auth: api key',
      '  status: 401 Unauthorized',
      '  response: {"error":{"message":"Incorrect API key provided: sk-stub"}}',
      'Hint: check network connectivity, DNS/TLS, that the base URL includes the API version (usually /v1), and that the model exists on the provider.'
    ].join('\n')

    const { ctx, failAssistantMessage } = errorContext(raw)

    handleStatusEvent(ctx)

    expect(failAssistantMessage).toHaveBeenCalledWith(
      'sess-1',
      raw,
      1_700_000_100,
      expect.objectContaining({ code: 'provider_key_rejected' })
    )
    const toast = $notifications.get()[0]
    expect(toast.message).toMatch(/^The provider rejected the key or login\./)
    expect(toast.message).not.toMatch(/endpoint|response|sk-stub|Hint/)
    expect(toast.detail).toContain('status: 401 Unauthorized')
    expect(toast.detail).not.toMatch(/Hint:/)
  })

  it('never makes an HTML body the toast message', () => {
    const raw = 'chat request failed\n  status: 418\n  response: <html><body>teapot</body></html>'
    const { ctx } = errorContext(raw)

    handleStatusEvent(ctx)

    const toast = $notifications.get()[0]
    expect(toast.message).not.toMatch(/<html|teapot/)
    expect(toast.detail).toContain('<html>')
  })
})
