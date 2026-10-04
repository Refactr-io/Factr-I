import { afterEach, describe, expect, it } from 'vitest'

import {
  $externalSessions,
  $readOnlySessionIds,
  $sessionTitleOverrides,
  rememberExternalSession
} from './read-only-sessions'

afterEach(() => {
  $externalSessions.set({})
  $readOnlySessionIds.set(new Set())
  $sessionTitleOverrides.set({})
})

describe('rememberExternalSession', () => {
  it('names a cron run from its resume reply and makes it a read-only transcript', () => {
    rememberExternalSession('run-1', { cwd: '/tmp', source: 'cron', title: 'Nightly backup · 3:00 AM' })

    expect($sessionTitleOverrides.get()['run-1']).toBe('Nightly backup · 3:00 AM')
    expect($readOnlySessionIds.get().has('run-1')).toBe(true)
    expect($externalSessions.get()['run-1']?.source).toBe('cron')
  })

  it('keeps an ordinary session writable and ignores an empty title', () => {
    rememberExternalSession('chat-1', { source: 'desktop', title: 'Notes' })
    rememberExternalSession('chat-2', { source: 'cron', title: '   ' })

    expect($readOnlySessionIds.get().has('chat-1')).toBe(false)
    expect($sessionTitleOverrides.get()['chat-2']).toBeUndefined()
  })
})
