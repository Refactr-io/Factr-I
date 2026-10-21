import { describe, expect, it } from 'vitest'

import { shouldResetVoiceForSession } from './use-composer-voice'

describe('shouldResetVoiceForSession', () => {
  it('resets when leaving a real session for another one or for a blank chat', () => {
    expect(shouldResetVoiceForSession('a', 'b')).toBe(true)
    expect(shouldResetVoiceForSession('a', null)).toBe(true)
    expect(shouldResetVoiceForSession('a', undefined)).toBe(true)
  })

  it('keeps the call when a blank chat gets its first session or nothing changed', () => {
    expect(shouldResetVoiceForSession(null, 'a')).toBe(false)
    expect(shouldResetVoiceForSession(undefined, 'a')).toBe(false)
    expect(shouldResetVoiceForSession('a', 'a')).toBe(false)
  })
})
