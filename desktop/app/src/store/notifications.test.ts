import { beforeEach, expect, test } from 'vitest'

import { en } from '@/i18n/en'

import { $notifications, clearNotifications, isDiskFullErrorMessage, notify, notifyError } from './notifications'
import { $backendRestartRequest, $routeRequest } from './recovery-requests'

beforeEach(() => {
  clearNotifications()
})

function lastMessage(): string {
  return $notifications.get()[0]?.message ?? ''
}

// Regression for #39365: a gateway auth 401 (bad API_SERVER_KEY) must not be
// summarized as a provider (OpenAI/OpenRouter) API key problem. The toast says
// "sign in again" in plain words and opens Gateways — no env-var names.
test('gateway_auth_failed error is summarized as sign-in, with an Open Gateways action', () => {
  notifyError(
    new Error(
      '401 {"error": {"message": "Invalid gateway API key (API_SERVER_KEY)", "type": "gateway_auth_error", "code": "gateway_auth_failed"}}'
    ),
    'Request failed'
  )

  expect(lastMessage()).not.toMatch(/API_SERVER_KEY|OpenAI|authentication failed/i)

  const action = $notifications.get()[0]?.action
  expect(action?.label).toBe(en.notifications.actions.openGateways)
  action?.onClick()
  expect($routeRequest.get()?.path).toBe('/settings?tab=gateway')
})

test('provider invalid_api_key error maps to the OpenAI summary and deep-links to Keys', () => {
  notifyError(
    new Error('401 {"error": {"message": "Incorrect API key provided", "code": "invalid_api_key"}}'),
    'Request failed'
  )

  expect(lastMessage()).not.toMatch(/401|invalid_api_key/)
  $notifications.get()[0]?.action?.onClick()
  expect($routeRequest.get()?.path).toBe('/settings?tab=keys&key=OPENAI_API_KEY')
})

test('"No STT provider available" toasts plain copy with an Open Keys action for Groq', () => {
  notifyError(
    new Error(
      'No STT provider available. Install faster-whisper for free local transcription, configure FACTR_LOCAL_STT_COMMAND or install a local whisper CLI, set GROQ_API_KEY for free Groq Whisper.'
    ),
    'Voice transcription failed'
  )

  expect(lastMessage()).toMatch(/speech-to-text key/)
  expect(lastMessage()).not.toMatch(/faster-whisper|FACTR_LOCAL_STT_COMMAND/)
  $notifications.get()[0]?.action?.onClick()
  expect($routeRequest.get()?.path).toBe('/settings?tab=keys&key=GROQ_API_KEY')
})

test('ELEVENLABS_API_KEY not set toasts plain copy with an Open Keys action for that key', () => {
  notifyError(new Error('ELEVENLABS_API_KEY not set'), 'Voice failed')

  expect(lastMessage()).not.toMatch(/ELEVENLABS_API_KEY|STT/)
  $notifications.get()[0]?.action?.onClick()
  expect($routeRequest.get()?.path).toBe('/settings?tab=keys&key=ELEVENLABS_API_KEY')
})

test('structured storage_* error codes route to Maintenance', () => {
  notifyError(new Error('500 {"detail":{"message":"database is locked","code":"storage_locked"}}'), 'Prompt failed')

  $notifications.get()[0]?.action?.onClick()
  expect($routeRequest.get()?.path).toBe('/command-center?section=maintenance')
})

test('405 method-not-allowed toasts a restart in plain words with a Restart Factr action', () => {
  const before = $backendRestartRequest.get()
  notifyError(new Error('405 Method Not Allowed'), 'Request failed')

  expect(lastMessage()).not.toMatch(/405|Method Not Allowed|backend/i)
  expect($notifications.get()[0]?.action?.label).toBe(en.notifications.actions.restartFactr)
  $notifications.get()[0]?.action?.onClick()
  expect($backendRestartRequest.get()).toBe(before + 1)
})

test('disk-full / ENOSPC phrasings are classified as disk-full, other storage failures are not', () => {
  expect(isDiskFullErrorMessage('OSError: [Errno 28] No space left on device')).toBe(true)
  expect(isDiskFullErrorMessage('sqlite3.OperationalError: database or disk is full')).toBe(true)
  expect(isDiskFullErrorMessage('disk full: session storage could not be written — free some disk space')).toBe(true)
  expect(isDiskFullErrorMessage('This is often a full disk — free some space')).toBe(true)
  expect(isDiskFullErrorMessage('session storage could not be written: permission denied')).toBe(false)
  expect(isDiskFullErrorMessage('network timeout')).toBe(false)
})

test('code-skew 503 unwraps to a restart-required summary, not raw IPC JSON', () => {
  notifyError(
    new Error(
      'Error invoking remote method \'factr:api\': Error: 503: {"detail":"Restart required: This process is running code from 08b4875f4a but the checkout on disk is now 48d2528066."}'
    ),
    'Could not load models'
  )

  expect(lastMessage()).not.toMatch(/factr:api|systemctl|backend/i)
  const before = $backendRestartRequest.get()
  expect($notifications.get()[0]?.action?.label).toBe(en.notifications.actions.restartFactr)
  $notifications.get()[0]?.action?.onClick()
  expect($backendRestartRequest.get()).toBe(before + 1)
})

test('no provider connected reads as a plain sentence with an Open Providers action, raw text as detail', () => {
  const raw = 'Claude credentials not available. Run `factr login claude` first.'

  notifyError(new Error(raw), 'Request failed')

  expect(lastMessage()).toBe(
    `${en.assistant.thread.errorCodes.no_provider.title}. ${en.assistant.thread.errorCodes.no_provider.body}`
  )
  expect($notifications.get()[0]?.detail).toBe(raw)
  const action = $notifications.get()[0]?.action
  expect(action?.label).toBe(en.assistant.thread.errorOpenProviders)
  action?.onClick()
  expect($routeRequest.get()?.path).toBe('/settings?tab=providers')
})

test('an HTTP 404 outside a provider call is not read as a missing model', () => {
  notifyError(new Error('HTTP 404: session not found'), 'Request failed')

  expect(lastMessage()).not.toMatch(/model/i)
})

test('a plugin error toast with the raw no-credentials text reads plain, with Open Providers and no CLI command', () => {
  const raw = 'Claude credentials not available. Run `factr login claude` first.'

  notify({ kind: 'error', message: raw })

  const [toast] = $notifications.get()
  expect(toast?.message).not.toMatch(/Claude|factr login/)
  expect(toast?.message).toBe(
    `${en.assistant.thread.errorCodes.no_provider.title}. ${en.assistant.thread.errorCodes.no_provider.body}`
  )
  expect(toast?.detail).toBe(raw)
  toast?.action?.onClick()
  expect($routeRequest.get()?.path).toBe('/settings?tab=providers')
})

test('a non-error toast and an unrelated error toast are left as written', () => {
  notify({ kind: 'info', message: 'Claude credentials not available.' })
  expect(lastMessage()).toBe('Claude credentials not available.')
  notify({ kind: 'error', message: 'Could not save the avatar.' })
  expect(lastMessage()).toBe('Could not save the avatar.')
})
