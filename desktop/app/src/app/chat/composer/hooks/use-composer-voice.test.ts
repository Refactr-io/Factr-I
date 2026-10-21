import { describe, expect, it } from 'vitest'

import { shouldResetVoiceForSession } from './use-composer-voice'

const base = { nextRuntime: 'b', nextStored: 'sb', owned: true, previousRuntime: 'a', previousStored: 'sa' }

describe('shouldResetVoiceForSession', () => {
  it('resets when a composer that owned voice moves to another conversation or a blank chat', () => {
    expect(shouldResetVoiceForSession(base)).toBe(true)
    expect(shouldResetVoiceForSession({ ...base, nextRuntime: null, nextStored: null })).toBe(true)
  })

  it('keeps the call across a runtime swap inside the same stored conversation', () => {
    expect(shouldResetVoiceForSession({ ...base, nextStored: 'sa' })).toBe(false)
  })

  it('keeps the call when a blank chat gets its first session, or nothing changed', () => {
    expect(shouldResetVoiceForSession({ ...base, previousRuntime: null, previousStored: null })).toBe(false)
    expect(shouldResetVoiceForSession({ ...base, nextRuntime: 'a', nextStored: 'sa' })).toBe(false)
  })

  it('never resets a composer that did not own voice (side tiles)', () => {
    expect(shouldResetVoiceForSession({ ...base, owned: false })).toBe(false)
  })
})
