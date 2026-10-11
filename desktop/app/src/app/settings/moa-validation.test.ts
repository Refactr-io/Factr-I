import { describe, expect, it } from 'vitest'

import { moaPresetNameProblem, moaProviderLacksCredentials } from './moa-validation'

describe('moaPresetNameProblem', () => {
  it('accepts plain names', () => {
    expect(moaPresetNameProblem('Fast mix-2', {})).toBeNull()
  })

  it('rejects empty, long, odd-character and duplicate names', () => {
    expect(moaPresetNameProblem('  ', {})).toBe('empty')
    expect(moaPresetNameProblem('x'.repeat(200), {})).toBe('long')
    expect(moaPresetNameProblem('a: {b}', {})).toBe('chars')
    expect(moaPresetNameProblem('mix', { mix: {} })).toBe('duplicate')
  })
})

describe('moaProviderLacksCredentials', () => {
  it('only flags providers explicitly unauthenticated', () => {
    expect(moaProviderLacksCredentials({ authenticated: false })).toBe(true)
    expect(moaProviderLacksCredentials({ authenticated: true })).toBe(false)
    expect(moaProviderLacksCredentials({})).toBe(false)
    expect(moaProviderLacksCredentials(undefined)).toBe(false)
  })
})
